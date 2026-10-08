# vellcast-relay: servidor de mídia para produção ou self-hosting, com o player web em /watch.
# Build: docker build -f infrastructure/docker/relay.Dockerfile -t vellcast-relay .
# Run:   docker run --rm -p 8788:8788 -e VELLCAST_RELAY_SECRET=<32+ bytes> vellcast-relay

FROM node:22-bookworm-slim AS player
WORKDIR /src/apps/discord-activity
COPY apps/discord-activity/package.json apps/discord-activity/package-lock.json ./
RUN npm ci
COPY apps/discord-activity/ ./
RUN npm run build

FROM rust:1-bookworm AS build
WORKDIR /src
# O workspace inteiro é copiado (os manifests de todos os membros são necessários);
# só o relay e suas dependências são compilados.
COPY . .
RUN cargo build --release --locked -p vellcast-relay --bin vellcast-relay

FROM debian:bookworm-slim
RUN useradd --system --uid 10001 vellcast
COPY --from=build /src/target/release/vellcast-relay /usr/local/bin/vellcast-relay
COPY --from=player /src/apps/discord-activity/dist /usr/share/vellcast/player
ENV VELLCAST_BIND=0.0.0.0:8788 \
    VELLCAST_STATIC_DIR=/usr/share/vellcast/player \
    VELLCAST_LOG_FORMAT=json \
    RUST_LOG=info
USER vellcast
EXPOSE 8788
HEALTHCHECK --interval=30s --timeout=3s --retries=3 \
  CMD ["/bin/bash", "-c", "exec 3<>/dev/tcp/127.0.0.1/8788 && printf 'GET /healthz HTTP/1.0\\r\\n\\r\\n' >&3 && grep -q ok <&3"]
# O relay não grava nada em disco: sem volumes.
CMD ["/usr/local/bin/vellcast-relay"]
