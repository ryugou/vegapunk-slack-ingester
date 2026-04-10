# Build stage
FROM rust:1.82-slim AS builder

RUN apt-get update && apt-get install -y protobuf-compiler && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY Cargo.toml Cargo.lock build.rs rust-toolchain.toml ./
COPY proto/ proto/
COPY src/ src/

RUN cargo build --release

# Runtime stage
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*

RUN useradd -r -s /bin/false ingester
USER ingester

COPY --from=builder /app/target/release/vegapunk-slack-ingester /app/vegapunk-slack-ingester

HEALTHCHECK --interval=30s --timeout=5s --retries=3 \
  CMD /app/vegapunk-slack-ingester health

ENTRYPOINT ["/app/vegapunk-slack-ingester"]
CMD ["serve"]
