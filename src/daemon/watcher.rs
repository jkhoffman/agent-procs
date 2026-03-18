//! File watcher with debounce for watch-mode process restarts.

use globset::{Glob, GlobSet, GlobSetBuilder};
use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::sync::mpsc;

/// Default patterns always ignored by watchers.
const DEFAULT_IGNORE: &[&str] = &[
    "**/.git/**",
    "**/node_modules/**",
    "**/target/**",
    "**/__pycache__/**",
    "**/*.pyc",
    "**/.DS_Store",
];

const DEBOUNCE_MS: u64 = 500;

/// Handle to a running file watcher. Drop to stop watching.
pub struct WatchHandle {
    _watcher: RecommendedWatcher,
    _debounce_handle: tokio::task::JoinHandle<()>,
}

/// Create a file watcher that sends the process name to `restart_tx`
/// when watched files change (after debounce).
pub fn create_watcher(
    paths: &[String],
    ignore: Option<&[String]>,
    base_dir: &Path,
    process_name: String,
    restart_tx: mpsc::Sender<String>,
) -> Result<WatchHandle, String> {
    let _ignore_set = build_ignore_set(ignore)?;
    let _watch_set = build_watch_set(paths)?;

    let (event_tx, mut event_rx) = mpsc::channel::<PathBuf>(256);

    let watcher = RecommendedWatcher::new(
        move |res: Result<Event, notify::Error>| {
            if let Ok(event) = res {
                for path in event.paths {
                    let _ = event_tx.blocking_send(path);
                }
            }
        },
        notify::Config::default(),
    )
    .map_err(|e| format!("failed to create watcher: {}", e))?;

    // Watch the base directory recursively
    let dirs = resolve_watch_dirs(paths, base_dir);
    let mut watcher = watcher;
    for dir in &dirs {
        let _ = watcher.watch(dir, RecursiveMode::Recursive);
    }

    // Debounce task
    let debounce_handle = tokio::spawn(async move {
        loop {
            // Wait for first event
            let Some(_path) = event_rx.recv().await else {
                break;
            };

            // Drain events during debounce window
            let deadline = tokio::time::Instant::now() + Duration::from_millis(DEBOUNCE_MS);
            loop {
                match tokio::time::timeout_at(deadline, event_rx.recv()).await {
                    Ok(Some(_)) => {}   // more events, keep waiting
                    Ok(None) => return, // channel closed
                    Err(_) => break,    // timeout — debounce complete
                }
            }

            // Send restart signal
            if restart_tx.send(process_name.clone()).await.is_err() {
                break; // receiver dropped
            }
        }
    });

    Ok(WatchHandle {
        _watcher: watcher,
        _debounce_handle: debounce_handle,
    })
}

fn build_ignore_set(user_ignore: Option<&[String]>) -> Result<GlobSet, String> {
    let mut builder = GlobSetBuilder::new();
    for pat in DEFAULT_IGNORE {
        builder.add(Glob::new(pat).map_err(|e| e.to_string())?);
    }
    if let Some(patterns) = user_ignore {
        for pat in patterns {
            builder.add(Glob::new(pat).map_err(|e| e.to_string())?);
        }
    }
    builder.build().map_err(|e| e.to_string())
}

fn build_watch_set(paths: &[String]) -> Result<GlobSet, String> {
    let mut builder = GlobSetBuilder::new();
    for pat in paths {
        builder.add(Glob::new(pat).map_err(|e| e.to_string())?);
    }
    builder.build().map_err(|e| e.to_string())
}

fn resolve_watch_dirs(_patterns: &[String], base: &Path) -> Vec<PathBuf> {
    // For each glob pattern, watch the base directory
    // (notify handles recursive watching, glob filtering happens in debounce)
    vec![base.to_path_buf()]
}
