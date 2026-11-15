# syntax=docker/dockerfile:1

# Toolchain shared by every stage that compiles Rust. The versions here
# must match rust-toolchain.toml and the wasm-bindgen crate version.
FROM rust:1.97.1-bookworm AS toolchain
RUN rustup target add wasm32-unknown-unknown
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    cargo install wasm-bindgen-cli --version 0.2.130 --locked

# The server binary and the wasm module, both from the same sources.
FROM toolchain AS rust-build
WORKDIR /src
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates crates
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked -p fb-server \
    && cargo build --release --locked -p fb-wasm --target wasm32-unknown-unknown \
    && mkdir -p /out/wasm \
    && cp target/release/fb-server /out/fb-server \
    && wasm-bindgen --target web --out-dir /out/wasm \
        target/wasm32-unknown-unknown/release/fb_wasm.wasm

FROM node:24-bookworm-slim AS web-build
WORKDIR /web
COPY web/package.json web/package-lock.json ./
RUN npm ci
COPY web .
COPY --from=rust-build /out/wasm src/wasm
RUN npm run build

# Runs the whole test suite: Rust tests, then the cross-target determinism
# harness in Node and the three browser engines, then the browser smoke
# tests against the real server.
FROM mcr.microsoft.com/playwright:v1.65.1-noble AS test
ENV CARGO_HOME=/usr/local/cargo \
    RUSTUP_HOME=/usr/local/rustup \
    PATH=/usr/local/cargo/bin:$PATH \
    CI=1
COPY --from=toolchain /usr/local/cargo /usr/local/cargo
COPY --from=toolchain /usr/local/rustup /usr/local/rustup
RUN apt-get update \
    && apt-get install -y --no-install-recommends build-essential \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY tests/e2e/package.json tests/e2e/package-lock.json tests/e2e/
RUN cd tests/e2e && npm ci
COPY . .
# No cache mounts here: the compiled tests and the crate sources have to be
# in the image so that running it needs no network and no rebuild.
RUN cargo test --workspace --release --no-run \
    && cd tests/e2e && mkdir -p pkg && npm run build
CMD ["sh", "-c", "cargo test --workspace --release && cd tests/e2e && npm test"]

# What `docker compose up` runs: the server, the built page and nothing else.
FROM debian:bookworm-slim AS runtime
RUN useradd --system --uid 10001 --no-create-home framebrawl \
    && mkdir /data \
    && chown framebrawl /data
COPY --from=rust-build /out/fb-server /usr/local/bin/fb-server
COPY --from=web-build /web/dist /srv/web
ENV FB_STATIC=/srv/web \
    FB_DATA=/data
USER framebrawl
VOLUME /data
EXPOSE 8080/tcp 7600/udp
CMD ["fb-server"]
