use crate::daemon::actor::{PmHandle, ProxyState};
use crate::error::ProxyError;
use crate::protocol::{ProcessState, Response};
use http_body_util::{BodyExt, Full};
use hyper::body::{Bytes, Incoming};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response as HyperResponse, StatusCode};
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use hyper_util::rt::TokioIo;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::watch;

type BoxBody = http_body_util::combinators::BoxBody<Bytes, hyper::Error>;

/// Bind an available port for the proxy listener, returning the bound listener.
/// Eliminates TOCTOU by keeping the listener alive between finding and using the port.
pub fn bind_proxy_port(explicit: Option<u16>) -> Result<(std::net::TcpListener, u16), ProxyError> {
    const PROXY_PORT_MIN: u16 = 9090;
    const PROXY_PORT_MAX: u16 = 9190;

    if let Some(port) = explicit {
        match std::net::TcpListener::bind(("127.0.0.1", port)) {
            Ok(listener) => return Ok((listener, port)),
            Err(source) => return Err(ProxyError::PortUnavailable { port, source }),
        }
    }

    for port in PROXY_PORT_MIN..=PROXY_PORT_MAX {
        if let Ok(listener) = std::net::TcpListener::bind(("127.0.0.1", port)) {
            return Ok((listener, port));
        }
    }

    Err(ProxyError::NoFreePort {
        min: PROXY_PORT_MIN,
        max: PROXY_PORT_MAX,
    })
}

/// Extract the subdomain from a Host header value.
///
/// - "api.localhost:9090" -> Some("api")
/// - "tenant.api.localhost:9090" -> Some("api") (second-to-last before "localhost")
/// - "localhost:9090" -> None
/// - "api.localhost" -> Some("api")
pub fn extract_subdomain(host: &str) -> Option<String> {
    // Strip port if present
    let hostname = host.split(':').next().unwrap_or(host);

    let parts: Vec<&str> = hostname.split('.').collect();
    // parts for "api.localhost" = ["api", "localhost"]
    // parts for "tenant.api.localhost" = ["tenant", "api", "localhost"]
    // parts for "localhost" = ["localhost"]

    if parts.len() < 2 {
        return None;
    }

    // The last part should be "localhost" (or the base domain)
    // The subdomain we want is the one immediately before "localhost"
    let second_to_last = parts[parts.len() - 2];
    if parts.last() == Some(&"localhost") && parts.len() >= 2 {
        // "localhost" alone means parts.len() == 1, already handled above
        Some(second_to_last.to_string())
    } else {
        None
    }
}

type HttpClient = Client<hyper_util::client::legacy::connect::HttpConnector, Incoming>;

/// Start the reverse proxy HTTP server using a pre-bound listener.
pub async fn start_proxy(
    std_listener: std::net::TcpListener,
    proxy_port: u16,
    handle: PmHandle,
    proxy_state_rx: watch::Receiver<ProxyState>,
    shutdown: Arc<tokio::sync::Notify>,
) -> std::io::Result<()> {
    std_listener.set_nonblocking(true)?;
    let listener = TcpListener::from_std(std_listener)?;

    // Single client instance shared across all requests (connection pool via Arc)
    let client: HttpClient = Client::builder(TokioExecutor::new()).build_http();

    loop {
        let (stream, _remote_addr) = tokio::select! {
            result = listener.accept() => match result {
                Ok(conn) => conn,
                Err(e) => {
                    tracing::warn!(error = %e, "proxy accept error");
                    continue;
                }
            },
            () = shutdown.notified() => {
                return Ok(());
            }
        };

        let handle = handle.clone();
        let proxy_state_rx = proxy_state_rx.clone();
        let client = client.clone();
        let pp = proxy_port;

        tokio::spawn(async move {
            let io = TokioIo::new(stream);
            let client = client.clone();
            let svc = service_fn(move |req: Request<Incoming>| {
                let handle = handle.clone();
                let proxy_state_rx = proxy_state_rx.clone();
                let client = client.clone();
                async move { handle_proxy_request(req, &handle, &proxy_state_rx, client, pp).await }
            });

            if let Err(e) = http1::Builder::new()
                .serve_connection(io, svc)
                .with_upgrades()
                .await
            {
                // Connection errors are normal (client disconnects, etc.)
                if !e.is_incomplete_message() {
                    tracing::warn!(error = %e, "proxy connection error");
                }
            }
        });
    }
}

/// Handle an incoming proxy request by routing it to the appropriate backend process.
async fn handle_proxy_request(
    req: Request<Incoming>,
    handle: &PmHandle,
    proxy_state_rx: &watch::Receiver<ProxyState>,
    client: HttpClient,
    proxy_port: u16,
) -> Result<HyperResponse<BoxBody>, hyper::Error> {
    // Extract subdomain from Host header
    let host = req
        .headers()
        .get(hyper::header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    let subdomain = extract_subdomain(host);

    let process_name = match subdomain {
        Some(name) => name,
        None => {
            // No subdomain -> serve status page
            return Ok(status_page(handle, proxy_port).await);
        }
    };

    // Lock-free port lookup via watch channel
    let backend_port = {
        let state = proxy_state_rx.borrow();
        state.port_map.get(&process_name).copied()
    };

    let backend_port = match backend_port {
        Some(port) => port,
        None => {
            let msg = format!(
                "502 Bad Gateway: no running process named '{}' with a port. Visit http://localhost:{} to see available routes.",
                process_name, proxy_port
            );
            return Ok(HyperResponse::builder()
                .status(StatusCode::BAD_GATEWAY)
                .body(text_body(msg))
                .unwrap());
        }
    };

    // Build the forwarded request
    let method = req.method().clone();
    let uri = req.uri().clone();
    let path_and_query = uri
        .path_and_query()
        .map_or("/", hyper::http::uri::PathAndQuery::as_str);
    let new_uri = format!("http://127.0.0.1:{}{}", backend_port, path_and_query);

    let mut builder = Request::builder().method(method).uri(&new_uri);

    // Copy headers, rewriting Host
    for (key, value) in req.headers() {
        if key == hyper::header::HOST {
            builder = builder.header(hyper::header::HOST, format!("127.0.0.1:{}", backend_port));
        } else {
            builder = builder.header(key, value);
        }
    }

    let forwarded_req = builder.body(req.into_body()).unwrap();

    match client.request(forwarded_req).await {
        Ok(resp) => {
            // Stream the response body through without buffering
            let (parts, body) = resp.into_parts();
            let boxed_body = body.boxed();
            Ok(HyperResponse::from_parts(parts, boxed_body))
        }
        Err(e) => {
            let msg = format!(
                "502 Bad Gateway: failed to connect to backend '{}' on port {}: {}",
                process_name, backend_port, e
            );
            Ok(HyperResponse::builder()
                .status(StatusCode::BAD_GATEWAY)
                .body(text_body(msg))
                .unwrap())
        }
    }
}

/// Convert a string into a `BoxBody` for error/status responses.
fn text_body(s: String) -> BoxBody {
    Full::new(Bytes::from(s))
        .map_err(|never| match never {})
        .boxed()
}

/// Generate a plain-text status page listing all routes.
async fn status_page(handle: &PmHandle, proxy_port: u16) -> HyperResponse<BoxBody> {
    let resp = handle.status_snapshot().await;
    let mut body = format!("agent-procs proxy on port {}\n\n", proxy_port);

    if let Response::Status { processes } = resp {
        if processes.is_empty() {
            body.push_str("No processes running.\n");
        } else {
            body.push_str("Routes:\n");
            for p in &processes {
                let state_str = match p.state {
                    ProcessState::Running => "running",
                    ProcessState::Exited => "exited",
                };
                use std::fmt::Write;
                if let Some(port) = p.port {
                    let _ = writeln!(
                        body,
                        "  http://{}.localhost:{} -> 127.0.0.1:{} [{}]",
                        p.name, proxy_port, port, state_str
                    );
                } else {
                    let _ = writeln!(body, "  {} (no port) [{}]", p.name, state_str);
                }
            }
        }
    }

    HyperResponse::builder()
        .status(StatusCode::OK)
        .header("content-type", "text/plain")
        .body(text_body(body))
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_subdomain_simple() {
        assert_eq!(extract_subdomain("api.localhost:9090"), Some("api".into()));
    }

    #[test]
    fn test_extract_subdomain_nested() {
        assert_eq!(
            extract_subdomain("tenant.api.localhost:9090"),
            Some("api".into())
        );
    }

    #[test]
    fn test_extract_subdomain_bare_localhost() {
        assert_eq!(extract_subdomain("localhost:9090"), None);
    }

    #[test]
    fn test_extract_subdomain_no_port() {
        assert_eq!(extract_subdomain("api.localhost"), Some("api".into()));
    }
}
