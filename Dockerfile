# Dockerfile — theOS Bootstrap Server for Railway.app
   FROM rust:1.75-slim as builder
   WORKDIR /app
   COPY . .
   RUN cargo build -p theos-daemon --release --target x86_64-unknown-linux-gnu

   FROM debian:bookworm-slim
   RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
   WORKDIR /app
   COPY --from=builder /app/target/x86_64-unknown-linux-gnu/release/theos-daemon /app/bootstrap

   # Set bootstrap mode + port
   ENV THEOS_MODE=bootstrap
   ENV THEOS_PORT=7700

   EXPOSE 7700/udp
   CMD ["/app/bootstrap"]