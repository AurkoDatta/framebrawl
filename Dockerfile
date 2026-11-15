# syntax=docker/dockerfile:1

# Toolchain shared by every stage that compiles Rust. The versions here
# must match rust-toolchain.toml and the wasm-bindgen crate version.
FROM rust:1.97.1-bookworm AS toolchain
RUN rustup target add wasm32-unknown-unknown
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    cargo install wasm-bindgen-cli --version 0.2.130 --locked

# Runs the whole test suite: Rust tests, then the cross-target determinism
# harness in Node and the three browser engines.
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
