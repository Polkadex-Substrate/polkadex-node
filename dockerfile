# This file is part of Polkadex.
#
# Copyright (c) 2021-2023 Polkadex oü.
# SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
#
# This program is free software: you can redistribute it and/or modify
# it under the terms of the GNU General Public License as published by
# the Free Software Foundation, either version 3 of the License, or
# (at your option) any later version.
#
# This program is distributed in the hope that it will be useful,
# but WITHOUT ANY WARRANTY; without even the implied warranty of
# MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
# GNU General Public License for more details.
#
# You should have received a copy of the GNU General Public License
# along with this program. If not, see <https://www.gnu.org/licenses/>.

# FROM bitnami/git:latest AS builder
#
# RUN apt-get update && apt-get install --assume-yes curl build-essential cmake clang jq protobuf-compiler ca-certificates
#
# RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y && \
#   export PATH="$PATH:$HOME/.cargo/bin" && \
#   rustup toolchain install nightly && \
#   rustup target add wasm32v1-none --toolchain nightly && \
#   rustup default nightly && \
#   git clone https://github.com/Polkadex-Substrate/polkadex-node -b master && \
#   cd polkadex-node && \
#   git checkout $(git describe --tags --abbrev=0) && \
#   cargo build --release -p polkadex-node
#
# # /\-Build Stage | Final Stage-\/
#
# FROM docker.io/library/ubuntu:20.04
# COPY --from=builder /Polkadex/target/release/polkadex-node /usr/local/bin
#
# RUN apt-get update && apt-get install --assume-yes curl ca-certificates
#
# RUN useradd -m -u 1000 -U -s /bin/sh -d /polkadex-node polkadex-node && \
#         mkdir -p /polkadex-node/.local/share && \
#         mkdir /data && \
#         chown -R polkadex-node:polkadex-node /data && \
#         ln -s /data /polkadex-node/.local/share/polkadex-node && \
#         rm -rf /usr/bin /usr/sbin
#
# COPY --from=builder /Polkadex/extras/customSpecRaw.json /data
#
# USER polkadex-node
# EXPOSE 30333 9933 9944
# VOLUME ["/data"]
#
# EXPOSE 30333 9933 9944
#
# ENTRYPOINT ["/usr/local/bin/polkadex-node"]

# You should be able to run a validator using this docker image in a bash environmment with the following command:
# docker run <docker_image_name> --chain /data/customSpecRaw.json $(curl -s https://raw.githubusercontent.com/Polkadex-Substrate/Polkadex/main/docs/run-a-validator.md | grep -o -m 1 -E "\-\-bootnodes \S*") --validator --name "Validator-Name"


# FROM rust:1.89-slim-bookworm AS builder
#
# RUN apt-get update && apt-get install -y --no-install-recommends \
#     curl cmake clang jq protobuf-compiler ca-certificates \
#     build-essential && \
#     rm -rf /var/lib/apt/lists/*
#
# WORKDIR /build
#
# RUN curl -fL https://codeload.github.com/Polkadex-Substrate/polkadex-node/tar.gz/refs/tags/v0.1.0 \
#     | tar xz && \
#     mv polkadex-node-* polkadex-node
#
# RUN cd polkadex-node && \
#     rustup toolchain install nightly && \
#     rustup target add wasm32v1-none --toolchain nightly && \
#     rustup default nightly && \
#     cargo build --release -p polkadex-node
#
# # Runtime stage
# FROM ubuntu:20.04
# RUN apt-get update && apt-get install -y --no-install-recommends \
#     curl ca-certificates && \
#     rm -rf /var/lib/apt/lists/* && \
#     useradd -m -u 1000 -U -s /bin/sh -d /polkadex-node polkadex-node && \
#     mkdir -p /polkadex-node/.local/share /data && \
#     chown -R polkadex-node:polkadex-node /data && \
#     ln -s /data /polkadex-node/.local/share/polkadex-node && \
#     rm -rf /usr/bin /usr/sbin
#
# COPY --from=builder /build/polkadex-node/target/release/polkadex-node /usr/local/bin/
# COPY --from=builder /build/polkadex-node/extras/customSpecRaw.json /data/
#
# USER polkadex-node
# EXPOSE 30333 9933 9944
# VOLUME ["/data"]
# ENTRYPOINT ["/usr/local/bin/polkadex-node"]


# ----------------------------
# Stage 1: Builder
# ----------------------------
FROM rust:1.91-slim-bookworm AS builder

# Install system dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    cmake clang jq protobuf-compiler ca-certificates build-essential curl \
    && rm -rf /var/lib/apt/lists/*

# Set working directory
WORKDIR /build/polkadex-node

# Copy the current repo into the build context
COPY . .

# Install Rust nightly and wasm target
RUN rustup toolchain install nightly \
    && rustup target add wasm32v1-none --toolchain nightly \
    && rustup default nightly

# Build the node in release mode
RUN cargo build --release -p polkadex-node

# ----------------------------
# Stage 2: Runtime
# ----------------------------
FROM debian:bookworm-slim

# Install minimal dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*

# Create a non-root user
RUN useradd -m -u 1000 -U -s /bin/sh -d /polkadex-node polkadex-node \
    && mkdir -p /data \
    && chown -R polkadex-node:polkadex-node /data

# Copy the compiled binary from builder stage
COPY --from=builder /build/polkadex-node/target/release/polkadex-node /usr/local/bin/

# Optional: copy local chain spec if it exists
COPY ./extras/customSpecRaw.json /data/

# Set working directory and permissions
WORKDIR /polkadex-node
RUN chown -R polkadex-node:polkadex-node /polkadex-node

# Switch to non-root user
USER polkadex-node

# Expose Polkadex node ports
EXPOSE 30333 9933 9944

# Data volume
VOLUME ["/data"]

# Entrypoint
ENTRYPOINT ["/usr/local/bin/polkadex-node"]

