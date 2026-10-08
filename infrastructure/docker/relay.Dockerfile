# vellcast-relay: servidor de mídia para produção ou self-hosting.
# Build: docker build -f infrastructure/docker/relay.Dockerfile -t vellcast-relay .
# Run:   docker run --rm -p 8788:8788 -e VELLCAST_RELAY_SECRET=<32+ bytes> vellcast-relay

FROM rust:1-bookworm AS build
WORKDIR /src
# O workspace inteiro é copiado (os manifests de todos os membros são necessários);
# só o relay e suas dependências são compilados.
COPY . .
RUN cargo build --release --locked -p vellcast-relay --bin vellcast-relay

FROM debian:bookworm-slim
RUN useradd --system --uid 10001 vellcast
COPY --from=build /src/target/release/vellcast-relay /usr/local/bin/vellcast-relay
ENV VELLCAST_BIND=0.0.0.0:8788 \
    VELLCAST_LOG_FORMAT=json \
    RUST_LOG=info
USER vellcast
EXPOSE 8788
# O relay não grava nada em disco: sem volumes.
CMD ["/usr/local/bin/vellcast-relay"]
