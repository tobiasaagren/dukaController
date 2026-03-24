# Build stage
FROM rust:bookworm AS builder
WORKDIR /app

# Cache dependencies: build a stub first so dep compilation is a separate layer
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo "fn main() {}" > src/main.rs \
    && cargo build --release \
    && rm -rf src

# Build the real binary
COPY src ./src
RUN touch src/main.rs && cargo build --release

# Runtime stage
FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=builder /app/target/release/duka .
COPY static ./static
COPY config.toml .

EXPOSE 3000
CMD ["./duka"]
