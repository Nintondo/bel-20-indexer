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

ARG GIT_USERNAME
ARG GIT_PERSONAL_ACCESS_TOKEN
ENV CARGO_NET_GIT_FETCH_WITH_CLI=true

RUN printf "machine github.com\nlogin %s\npassword %s\n" "$GIT_USERNAME" "$GIT_PERSONAL_ACCESS_TOKEN" > ~/.netrc && \
    chmod 600 ~/.netrc

COPY Cargo.toml ./
COPY src src

RUN cargo fetch && cargo build --release

RUN rm -rf ~/.cargo/git && \
    rm -rf ~/.cargo/registry && \
    rm -f ~/.netrc

FROM ubuntu:24.04 AS runner

WORKDIR /app

RUN apt update -y && \
    apt install -y curl openssl libc6 libgcc-s1 librocksdb-dev

COPY --from=builder /usr/src/app/target/release/bel_20_node .

EXPOSE 3001

ENTRYPOINT ["./bel_20_node"]