# Zero-crate Rust binary. Links system libsqlite3.

FROM rust:1-slim-bookworm AS builder
WORKDIR /build

RUN apt-get update \
    && apt-get install -y --no-install-recommends gcc libc6-dev libsqlite3-dev \
    && rm -rf /var/lib/apt/lists/*

COPY Cargo.toml Cargo.lock build.rs ./
COPY src ./src
COPY public ./public

RUN cargo build --release

FROM debian:bookworm-slim
WORKDIR /app
RUN apt-get update \
    && apt-get install -y --no-install-recommends libsqlite3-0 \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/almanac /usr/local/bin/almanac

ENV PORT=8000 \
    HOST=:: \
    DB_PATH=/app/data/calendar.db

EXPOSE 8000
CMD ["/usr/local/bin/almanac"]
