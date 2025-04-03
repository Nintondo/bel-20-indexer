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

COPY Cargo.toml ./
COPY src src
RUN cargo build --release

FROM debian:bookworm-slim AS runer

WORKDIR /app

RUN apt update -y && apt install -y curl openssl librocksdb-dev

COPY --from=builder /usr/src/app/target/release/bel_20_indexer .

EXPOSE 3001

ENTRYPOINT ["./bel_20_indexer"]

