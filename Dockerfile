# Multi-stage: compile a release binary, ship it on distroless/cc.
# rusqlite bundles SQLite (no libsqlite3 at runtime) but links glibc.

FROM rust:1-slim-bookworm AS builder
WORKDIR /build

RUN apt-get update \
    && apt-get install -y --no-install-recommends gcc libc6-dev \
    && rm -rf /var/lib/apt/lists/*

COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY public ./public

RUN cargo build --release \
    && ldd target/release/almanac \
    && if ldd target/release/almanac | grep -qi sqlite; then \
         echo "FAIL: external libsqlite3 dynamic dependency present" >&2; exit 1; \
       fi

FROM gcr.io/distroless/cc-debian12
WORKDIR /app
COPY --from=builder /build/target/release/almanac /usr/local/bin/almanac

ENV PORT=8000 \
    HOST=:: \
    DB_PATH=/app/data/calendar.db

EXPOSE 8000
CMD ["/usr/local/bin/almanac"]
