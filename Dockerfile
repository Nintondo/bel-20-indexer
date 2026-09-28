FROM rust:1.92-trixie AS builder

WORKDIR /usr/src/app

RUN apt update -y && \
    apt install -y --no-install-recommends \
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

RUN cargo fetch \
  && cargo build --release

RUN rm -rf /usr/local/cargo/git && \
    rm -rf /usr/local/cargo/registry

FROM debian:trixie-slim AS runner

RUN apt update -y && \
    apt install -y --no-install-recommends \
    tini \
    gosu \
    curl \
    rsync \
    libc6 \
    libgcc-s1 \ 
    libstdc++6 \
    ca-certificates && \
    apt-get clean && \
    rm -rf /var/lib/apt/lists/*

RUN groupadd --gid 1001 appuser \
  && useradd --uid 1001 --gid appuser --home /home/appuser --shell /usr/sbin/nologin appuser \
  && mkdir -p /home/appuser/.cache \
  && chown -R 1001:1001 /home/appuser

WORKDIR /app

COPY --from=builder /usr/src/app/target/release/bel_20_node .

COPY entrypoint.sh /entrypoint.sh
RUN chmod +x /entrypoint.sh

EXPOSE 8000

HEALTHCHECK --interval=30s --timeout=5s --start-period=180s --retries=3 \
    CMD curl --fail --silent --show-error --max-time 4 http://127.0.0.1:8000/readyz >/dev/null || exit 1

ENTRYPOINT ["/usr/bin/tini", "--", "/entrypoint.sh"]
CMD ["/app/bel_20_node"]
