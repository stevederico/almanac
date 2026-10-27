# Zero-crate Rust binary. Links SQLCipher 4.18, built from the official tag.
# Bookworm's packages stop at 4.6.1.

FROM rust:1-slim-bookworm AS builder
WORKDIR /build

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        gcc libc6-dev make tcl libssl-dev curl ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# SQLCipher 4.7+ builds with autosetup: --with-tempstore and the EXTRA_INIT
# hooks are required, and the library installs as libsqlite3 (the symlink
# keeps build.rs's -lsqlcipher working).
ARG SQLCIPHER_VERSION=4.18.0
RUN curl -fsSL -o /tmp/sqlcipher.tar.gz \
        "https://github.com/sqlcipher/sqlcipher/archive/refs/tags/v${SQLCIPHER_VERSION}.tar.gz" \
    && tar -xzf /tmp/sqlcipher.tar.gz -C /tmp \
    && cd "/tmp/sqlcipher-${SQLCIPHER_VERSION}" \
    && ./configure --prefix=/usr/local --with-tempstore=yes \
        CFLAGS="-DSQLITE_HAS_CODEC -DSQLITE_EXTRA_INIT=sqlcipher_extra_init -DSQLITE_EXTRA_SHUTDOWN=sqlcipher_extra_shutdown" LDFLAGS="-lcrypto" \
    && make -j"$(nproc)" \
    && make install \
    && ln -sf libsqlite3.so /usr/local/lib/libsqlcipher.so \
    && ldconfig \
    && rm -rf /tmp/sqlcipher.tar.gz "/tmp/sqlcipher-${SQLCIPHER_VERSION}"

COPY Cargo.toml Cargo.lock build.rs ./
COPY src ./src
COPY public ./public

RUN cargo build --release

FROM debian:bookworm-slim
WORKDIR /app
RUN apt-get update \
    && apt-get install -y --no-install-recommends libssl3 \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /usr/local/lib/libsqlcipher.so /usr/local/lib/libsqlite3.so* /usr/local/lib/
RUN ldconfig

COPY --from=builder /build/target/release/almanac /usr/local/bin/almanac

ENV PORT=8000 \
    HOST=:: \
    DB_PATH=/app/data/calendar.db

EXPOSE 8000
CMD ["/usr/local/bin/almanac"]
