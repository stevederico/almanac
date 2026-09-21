# Zero-crate Rust binary. Links system libsqlcipher (4.6 from bookworm-backports;
# bookworm's own package is 3.4.1).

FROM rust:1-slim-bookworm AS builder
WORKDIR /build

RUN echo "deb http://deb.debian.org/debian bookworm-backports main" > /etc/apt/sources.list.d/backports.list \
    && apt-get update \
    && apt-get install -y --no-install-recommends gcc libc6-dev \
    && apt-get install -y --no-install-recommends -t bookworm-backports libsqlcipher-dev \
    && rm -rf /var/lib/apt/lists/*

COPY Cargo.toml Cargo.lock build.rs ./
COPY src ./src
COPY public ./public

RUN cargo build --release

FROM debian:bookworm-slim
WORKDIR /app
RUN echo "deb http://deb.debian.org/debian bookworm-backports main" > /etc/apt/sources.list.d/backports.list \
    && apt-get update \
    && apt-get install -y --no-install-recommends -t bookworm-backports libsqlcipher1 \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/almanac /usr/local/bin/almanac

ENV PORT=8000 \
    HOST=:: \
    DB_PATH=/app/data/calendar.db

EXPOSE 8000
CMD ["/usr/local/bin/almanac"]
