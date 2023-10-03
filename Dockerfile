FROM rust:1.72-buster AS builder

WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY ./src/ ./src/
RUN cargo install --path .

FROM debian:buster-slim

COPY --from=builder /app/target/release/kitops /usr/local/bin/kitops
RUN apt-get update && apt-get install -y ca-certificates openssl && rm -rf /var/lib/apt/lists/*

ENTRYPOINT ["/usr/local/bin/kitops"]