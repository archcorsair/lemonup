# syntax=docker/dockerfile:1

# LemonUp container image. Works with Docker and Podman.
#
#   docker build -t lemonup .
#   podman build -t lemonup .
#
# The Rust version matches mise.toml and the CI workflows.

ARG RUST_VERSION=1.94.1
ARG DEBIAN_SUITE=trixie

FROM rust:${RUST_VERSION}-${DEBIAN_SUITE} AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked -p lemonup-app --bin lemonup \
 && cp target/release/lemonup /lemonup

FROM debian:${DEBIAN_SUITE}-slim

COPY --from=build /lemonup /usr/local/bin/lemonup

# /wow is where you bind mount your World of Warcraft folder.
# /state holds LemonUp's settings, linked addons and logs. Mount a volume there so they
# survive between runs. It is world-writable so the container can run as any user (use
# --user to match the owner of your WoW folder).
RUN mkdir -p /wow /state && chmod 0777 /state

ENV HOME=/state \
    XDG_CONFIG_HOME=/state/config \
    XDG_DATA_HOME=/state/data \
    XDG_CACHE_HOME=/state/cache

USER 1000:1000
WORKDIR /wow
ENTRYPOINT ["lemonup"]
