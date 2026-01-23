FROM rust:1.91.1-trixie AS builder

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
COPY packages packages

RUN cargo fetch && cargo build --release

RUN rm -rf /usr/local/cargo/git && \
    rm -rf /usr/local/cargo/registry

FROM debian:trixie-slim AS runner

WORKDIR /app

RUN apt update -y && \
    apt install -y --no-install-recommends \
    curl \
    rsync \
    libc6 \
    libgcc-s1 \ 
    libstdc++6 && \
    apt-get clean && \
    rm -rf /var/lib/apt/lists/* && \
    groupadd --gid 1001 appuser && \
    useradd --uid 1001 --gid 1001 --create-home appuser

COPY --from=builder /usr/src/app/target/release/bel_20_node .

RUN chown -R 1001:1001 /app
USER 1001

EXPOSE 8000

CMD ["./bel_20_node"]
