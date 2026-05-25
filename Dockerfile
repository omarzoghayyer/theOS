# Dockerfile — theOS Bootstrap Server for Railway.app
# Production-grade DHT bootstrap with health checks

FROM rust:1.75-slim as builder

WORKDIR /app

# Copy workspace
COPY . .

# Build bootstrap server (release mode)
RUN cargo build -p theos-daemon --release --target x86_64-unknown-linux-gnu 2>&1 | grep -E "Finished|error"

# Runtime stage
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy binary from builder
COPY --from=builder /app/target/x86_64-unknown-linux-gnu/release/theos-daemon /app/bootstrap

# Health check
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD curl -f http://localhost:7700/health || exit 1

# Run bootstrap server
EXPOSE 7700
CMD ["/app/bootstrap"]
