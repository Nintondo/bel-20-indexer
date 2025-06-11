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
    protobuf-compiler && \
    apt-get clean && \
    rm -rf /var/lib/apt/lists/*

COPY Cargo.toml ./
COPY src src

RUN cargo fetch && cargo build --release

RUN rm -rf ~/.cargo/git && \
    rm -rf ~/.cargo/registry

FROM ubuntu:24.04 AS runner

WORKDIR /app

RUN apt update -y && \
    apt install -y curl openssl libc6 libgcc-s1 librocksdb-dev && \
    apt-get clean && \
    rm -rf /var/lib/apt/lists/*

COPY --from=builder /usr/src/app/target/release/bel_20_node .

EXPOSE 8000

ENTRYPOINT ["./bel_20_node"]