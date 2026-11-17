# syntax=docker/dockerfile:1
# Build context: repository root (the Cargo workspace).
FROM rust:1-slim-trixie AS base
WORKDIR /app

# --- dev: source is bind-mounted, rebuilds on change ---
FROM base AS dev
RUN cargo install cargo-watch --locked
ENV CARGO_TARGET_DIR=/target
EXPOSE 8080
CMD ["cargo", "watch", "-w", "Cargo.toml", "-w", "src", "-x", "run -p lilanaya-api"]

# --- build: release binary ---
FROM base AS build
COPY Cargo.toml Cargo.lock ./
COPY src/app/api src/app/api
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/app/target \
    cargo build --release --locked -p lilanaya-api \
    && cp target/release/lilanaya-api /usr/local/bin/lilanaya-api

# --- runtime ---
FROM debian:trixie-slim AS runtime
COPY --from=build /usr/local/bin/lilanaya-api /usr/local/bin/lilanaya-api
USER nobody
EXPOSE 8080
CMD ["lilanaya-api"]
