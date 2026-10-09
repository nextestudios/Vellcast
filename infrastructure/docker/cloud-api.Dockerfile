# vellcast-cloud-api: contas, planos, assinaturas e entitlements (serviço comercial opcional).
# Build: docker build -f infrastructure/docker/cloud-api.Dockerfile -t vellcast-cloud-api .
# Run:   docker run --rm -p 8789:8789 -v vellcast-cloud:/data \
#          -e VELLCAST_CLOUD_ENTITLEMENT_KEY=<32+ bytes> vellcast-cloud-api

FROM rust:1-bookworm AS build
WORKDIR /src
COPY . .
RUN cargo build --release --locked -p vellcast-cloud-api --bin vellcast-cloud-api

FROM debian:bookworm-slim
RUN useradd --system --uid 10001 vellcast && mkdir -p /data && chown vellcast /data
COPY --from=build /src/target/release/vellcast-cloud-api /usr/local/bin/vellcast-cloud-api
ENV VELLCAST_CLOUD_BIND=0.0.0.0:8789 \
    VELLCAST_CLOUD_DB=/data/vellcast-cloud.db \
    RUST_LOG=info
USER vellcast
VOLUME ["/data"]
EXPOSE 8789
HEALTHCHECK --interval=30s --timeout=3s --retries=3 \
  CMD ["/bin/bash", "-c", "exec 3<>/dev/tcp/127.0.0.1/8789 && printf 'GET /healthz HTTP/1.0\\r\\n\\r\\n' >&3 && grep -q ok <&3"]
CMD ["/usr/local/bin/vellcast-cloud-api"]
