# Stage 1: Build
FROM rust:1.85-slim AS builder
WORKDIR /app
COPY . .
RUN cargo build --release

# Stage 2: Runtime
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/agent-procs /usr/local/bin/
ENTRYPOINT ["agent-procs"]
