FROM rust:1.85.1-bookworm AS builder

WORKDIR /usr/src/app

RUN apt update -y && \
    apt install -y \
    pkg-config \
    libssl-dev \
    git \
    build-essential \
    clang \
    libclang-dev \
    protobuf-compiler

ARG CARGO_TOKEN
ENV CARGO_NET_GIT_FETCH_WITH_CLI=true
RUN cargo login ${CARGO_TOKEN}

COPY Cargo.toml ./
COPY src src

RUN cargo fetch

RUN cargo build --release

FROM ubuntu:24.04 AS runner

WORKDIR /app

RUN apt update -y && \
    apt install -y curl openssl libc6 libgcc-s1 librocksdb-dev

COPY --from=builder /usr/src/app/target/release/bel_20_node .

EXPOSE 3001

ENTRYPOINT ["./bel_20_node"]