# Servidor de validação da ETAPA 0 + build da Discord Activity + mídia sintética.
# Build: docker build -f infrastructure/docker/stage0.Dockerfile -t vellcast-stage0 .
# Para a Activity funcionar dentro do Discord: --build-arg VITE_DISCORD_CLIENT_ID=<id>

FROM node:22-bookworm-slim AS activity
ARG VITE_DISCORD_CLIENT_ID=""
ENV VITE_DISCORD_CLIENT_ID=${VITE_DISCORD_CLIENT_ID}
WORKDIR /src/apps/discord-activity
COPY apps/discord-activity/package.json apps/discord-activity/package-lock.json ./
RUN npm ci
COPY apps/discord-activity/ ./
RUN npm run build

FROM rust:1-bookworm AS server
WORKDIR /src
# O workspace inteiro é copiado (os manifests de todos os membros são necessários);
# só o pacote pedido e suas dependências são compilados.
COPY . .
RUN cargo build --release --locked -p vellcast-stage0-server

# ffmpeg só existe neste estágio; não é distribuído na imagem final.
FROM debian:bookworm-slim AS media
RUN apt-get update && apt-get install -y --no-install-recommends ffmpeg && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY scripts/generate-test-media.sh scripts/
RUN ./scripts/generate-test-media.sh

FROM debian:bookworm-slim
RUN useradd --system --uid 10001 vellcast
WORKDIR /app
COPY --from=server /src/target/release/vellcast-stage0-server /app/
COPY --from=activity /src/apps/discord-activity/dist /app/dist
COPY --from=media /src/tests/fixtures/media /app/media
ENV VELLCAST_BIND=0.0.0.0:8787 \
    VELLCAST_MEDIA_DIR=/app/media \
    VELLCAST_STATIC_DIR=/app/dist \
    RUST_LOG=info
USER vellcast
EXPOSE 8787
CMD ["/app/vellcast-stage0-server"]
