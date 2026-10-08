# Deployment — Vellcast

> Status: **Etapa 0**. Só o servidor de validação (`vellcast-stage0-server`) existe. O relay de produção (`services/relay`) é da Etapa 3; as seções marcadas como futuras descrevem o plano.

## 1. Execução local (Etapa 0)

Pré-requisitos: Rust estável, Node.js, FFmpeg (apenas para gerar mídia de teste).

```bash
# 1. Gerar mídia sintética (H.264 Annex B + Opus) em media/
./scripts/generate-test-media.sh

# 2. Build da Activity
cd apps/discord-activity && npm ci && npm run build && cd ../..

# 3. Servidor (serve a Activity + WebSocket de mídia)
cargo run --release -p vellcast-stage0-server
```

Abrir `http://localhost:8787` no navegador para testar fora do Discord.

Variáveis de ambiente:

| Variável | Padrão local | Padrão na imagem Docker | Descrição |
|---|---|---|---|
| `VELLCAST_BIND` | `127.0.0.1:8787` | `0.0.0.0:8787` | Endereço e porta HTTP/WebSocket |
| `VELLCAST_MEDIA_DIR` | `tests/fixtures/media` | `/app/media` | Diretório com a mídia de teste gerada |
| `VELLCAST_STATIC_DIR` | `apps/discord-activity/dist` | `/app/dist` | Build da Activity servida em `/` |
| `RUST_LOG` | `info,vellcast_stage0_server=debug` | `info` | Nível de log (`tracing`) |

O servidor serve a Activity e o WebSocket na **mesma origem**, o que simplifica o mapeamento de URL do Discord.

## 2. Testar dentro do Discord (túnel de desenvolvimento)

O Discord só carrega Activities via HTTPS público, através do proxy `https://<CLIENT_ID>.discordsays.com`.

1. Criar uma aplicação no [Discord Developer Portal](https://discord.com/developers/applications) e habilitar **Activities**.
2. Expor o servidor local com um túnel HTTPS, por exemplo:
   ```bash
   cloudflared tunnel --url http://localhost:8787
   ```
   (gera uma URL `https://<algo>.trycloudflare.com`; o túnel rápido é efêmero e só serve para testes).
3. Em **Activities → URL Mappings**, mapear o prefixo `/` para o host do túnel (sem `https://`), ex.: `/` → `algo.trycloudflare.com`.
4. Definir `VITE_DISCORD_CLIENT_ID` no build da Activity (ver `DISCORD_INTEGRATION.md`) e refazer o build.
5. Iniciar a Activity em um canal de voz de um servidor de teste (modo desenvolvedor/teste habilitado na aplicação).

Dentro do Discord, a página conecta em `wss://<CLIENT_ID>.discordsays.com/ws`; o proxy encaminha para o host mapeado. Requisições para hosts não mapeados falham com `blocked:csp`. O proxy só suporta WebSocket (não WebRTC/WebTransport).

**Segurança:** não commitar Client Secret, tokens ou URLs de túnel pessoais. Segredos vão em variáveis de ambiente.

## 3. Docker

Arquivo: `infrastructure/docker/stage0.Dockerfile` (multi-stage). Estrutura esperada:

1. **Estágio `web`** (Node): `npm ci` e `npm run build` em `apps/discord-activity` → `dist/`.
2. **Estágio `media`**: gera mídia sintética com FFmpeg (apenas no build; FFmpeg não vai para a imagem final).
3. **Estágio `build`** (Rust): `cargo build --release -p vellcast-stage0-server`.
4. **Estágio final** (imagem mínima, ex.: `debian:bookworm-slim` ou distroless): copia o binário, `dist/` e `media/`; roda como usuário não-root; expõe `8787`.

```bash
docker build -f infrastructure/docker/stage0.Dockerfile -t vellcast-stage0 .
docker run --rm -p 8787:8787 vellcast-stage0
```

Health check: `GET /healthz` → `200 ok`.

Imagens publicadas devem fixar versões das imagens base (tag + digest) e ser reconstruídas para atualizações de segurança.

## 4. VPS + Dokploy

1. VPS Linux com Docker e Dokploy instalados; DNS `A/AAAA` de um subdomínio (ex.: `stage0.seudominio.com`) apontando para a VPS.
2. No Dokploy: **Create Application** → fonte GitHub (`nextestudios/Vellcast`), branch desejada.
3. Build type: **Dockerfile**, caminho `infrastructure/docker/stage0.Dockerfile`, contexto `.` (raiz).
4. Variáveis de ambiente em **Environment** (`RUST_LOG=info`; `VELLCAST_BIND=0.0.0.0:8787`).
5. **Domains**: adicionar o domínio, container port `8787`, HTTPS habilitado com Let's Encrypt. O Traefik do Dokploy termina TLS e encaminha WebSocket automaticamente.
6. Configurar health check para `/healthz`.
7. Deploy. Verificar `https://stage0.seudominio.com/healthz` e abrir a página.
8. No Discord Developer Portal, mapear `/` → `stage0.seudominio.com`.

## 5. TLS e proxy reverso

- TLS sempre terminado no proxy (Traefik/Caddy/Nginx). O servidor escuta HTTP apenas na rede interna/localhost.
- WebSocket exige encaminhamento do upgrade. Em Nginx:
  ```nginx
  location /ws {
      proxy_pass http://127.0.0.1:8787;
      proxy_http_version 1.1;
      proxy_set_header Upgrade $http_upgrade;
      proxy_set_header Connection "upgrade";
      proxy_set_header Host $host;
      proxy_read_timeout 120s;   # maior que o intervalo de heartbeat
      proxy_buffering off;
  }
  ```
- Caddy e Traefik encaminham upgrade de WebSocket por padrão.
- Ajustar timeouts de inatividade do proxy acima do intervalo de heartbeat do protocolo.
- Desativar buffering/compressão de resposta para o caminho de mídia (dados já comprimidos; buffering aumenta latência).

## 6. Futuro: relay de produção e QUIC (Etapas 3 e 6)

- `services/relay` terá Dockerfile e `docker-compose.yml` de exemplo próprios para self-hosting, sem dependência do Vellcast Cloud.
- **QUIC (Quinn)**, se adotado entre desktop e relay, usa **UDP** (ex.: 443/udp ou porta dedicada):
  - Liberar a porta UDP no firewall da VPS e no provedor.
  - **Não presumir que o proxy reverso encaminha QUIC.** O Traefik/Nginx do Dokploy normalmente termina HTTP(S)/TCP; o QUIC do relay provavelmente precisará de porta UDP publicada diretamente no container (`ports: "4433:4433/udp"`) com certificado TLS próprio montado no container.
  - A Activity continua usando WebSocket via proxy do Discord.
- Capacidade: dimensionar pela banda de saída (ver `MONETIZATION.md` §4); o relay não precisa de GPU.
- Monitoramento: logs estruturados (JSON), `/healthz`, métricas agregadas (S-014).
