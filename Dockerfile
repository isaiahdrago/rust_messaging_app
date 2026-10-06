# --- Build stage ---
FROM rust:1.85-bookworm AS builder
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY migrations ./migrations
COPY html ./html
RUN cargo build --locked --release

# --- Runtime stage ---
FROM debian:bookworm-slim
WORKDIR /app

RUN apt-get update \
    && apt-get install --no-install-recommends -y ca-certificates wget \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --create-home --uid 10001 appuser \
    && mkdir /data \
    && chown appuser:appuser /data

COPY --from=builder /app/target/release/rust_messaging_app /app/server

USER appuser

EXPOSE 3000
ENV HOST=0.0.0.0 \
    PORT=3000 \
    DB_PATH=/data/chat.sqlite3

CMD ["/app/server"]
