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

## 6. Relay de mídia (`vellcast-relay`, ETAPA 3)

O relay é o servidor de produção e de self-hosting: salas privadas, convites, tokens e distribuição sem recodificação. Não precisa de GPU, não grava mídia em disco e não depende do Vellcast Cloud.

### 6.1 Execução local

```bash
export VELLCAST_RELAY_SECRET=$(openssl rand -hex 32)
VELLCAST_BIND=127.0.0.1:8788 cargo run --release -p vellcast-relay
```

Para desenvolvimento rápido, `VELLCAST_RELAY_INSECURE_DEV=1` gera um segredo aleatório a cada início (os tokens deixam de valer ao reiniciar). Sem segredo e sem essa flag, o relay **se recusa a iniciar**.

Publicar a mídia sintética numa sala:

```bash
curl -s -X POST localhost:8788/api/rooms -H 'content-type: application/json' -d '{"name":"teste"}'
# guarde ownerToken e publisherToken da resposta
VELLCAST_PUBLISHER_TOKEN=<publisherToken> cargo run --release -p vellcast-transport \
  --example publish_fixture -- ws://127.0.0.1:8788/ws 1080p60
```

Teste de carga: `node tests/benchmarks/relay-viewers.mjs http://127.0.0.1:8788 1080p60 20 30` (cria sala e convites, inicia o publicador e conecta N espectadores autenticados).

### 6.2 Variáveis de ambiente

| Variável | Padrão | Descrição |
|---|---|---|
| `VELLCAST_RELAY_SECRET` | — (obrigatória) | Segredo HMAC dos tokens, ≥ 32 bytes. Trocar o segredo invalida todos os tokens. |
| `VELLCAST_RELAY_INSECURE_DEV` | desligado | `1` gera segredo aleatório (apenas desenvolvimento). |
| `VELLCAST_BIND` | `127.0.0.1:8788` (`0.0.0.0:8788` na imagem) | Endereço HTTP/WebSocket. |
| `VELLCAST_CREATE_ROOM_KEY` | vazio | Se definida, `POST /api/rooms` exige `Authorization: Bearer <chave>`. Recomendado em servidores expostos. |
| `VELLCAST_TRUST_PROXY` | `false` | Usa o último endereço de `X-Forwarded-For` como IP do cliente (rate limit). Ative só atrás de proxy que sobrescreve esse cabeçalho. |
| `VELLCAST_MAX_ROOMS` | `1000` | Salas simultâneas. |
| `VELLCAST_MAX_VIEWERS_PER_ROOM` | `50` | Teto de espectadores por sala. |
| `VELLCAST_MAX_PUBLISHER_KBPS` | `40000` | Banda de entrada por transmissor. |
| `VELLCAST_PUBLISHER_GRACE_SECS` | `30` | Tolerância a queda do transmissor. |
| `VELLCAST_ROOM_TTL_SECS` | `86400` | Vida máxima de uma sala. |
| `VELLCAST_ROOM_IDLE_TTL_SECS` | `3600` | Sala sem transmissor e sem espectadores é removida após esse tempo. |
| `VELLCAST_VIEWER_TOKEN_TTL_SECS` | `43200` | Validade do token de espectador. |
| `VELLCAST_STATIC_DIR` | pasta `player` ao lado do executável, se existir | Player web (S-011) servido em `/` e `/watch`. Vazio desativa. Ver §6.8. |
| `VELLCAST_LOG_FORMAT` | texto (`json` na imagem) | `json` para logs estruturados. |
| `RUST_LOG` | `info` | Nível de log. |

Os logs nunca incluem tokens, convites, IPs de espectadores nem conteúdo de mídia.

### 6.3 API HTTP

| Método e caminho | Autorização | Resultado |
|---|---|---|
| `POST /api/rooms` `{name, maxViewers?}` | `VELLCAST_CREATE_ROOM_KEY`, se definida | `201 {room, ownerToken, publisherToken}` |
| `GET /api/rooms/{id}` | token da sala (qualquer papel) | estado, espectadores, limites |
| `POST /api/rooms/{id}/invites` `{maxUses?, ttlSecs?}` | dono | `201 {code, maxUses, expiresAt}` (o código aparece uma vez; o relay guarda só o hash) |
| `POST /api/rooms/{id}/join` `{invite}` | convite válido | `{viewerToken, room}` |
| `POST /api/rooms/{id}/publisher-token` | dono | novo token de transmissor |
| `DELETE /api/rooms/{id}` | dono | `204`; desconecta transmissor e espectadores |
| `GET /ws` | mensagem `Auth` | mídia (ver `STREAMING_PROTOCOL.md`) |
| `GET /healthz` | — | `ok` |
| `GET /metrics` | — | JSON agregado (salas, espectadores, bytes, descartes, falhas de autenticação). Sem conteúdo nem tokens. Restrinja no proxy se não quiser expor. |

Tokens vão sempre em `Authorization: Bearer`, nunca na URL.

### 6.4 Docker

```bash
docker build -f infrastructure/docker/relay.Dockerfile -t vellcast-relay .
docker run --rm -p 127.0.0.1:8788:8788 -e VELLCAST_RELAY_SECRET=$(openssl rand -hex 32) vellcast-relay
```

Self-hosting com Compose: copie `infrastructure/docker/relay.env.example` para `relay.env` (ignorado pelo git), preencha o segredo e rode `docker compose -f infrastructure/docker/docker-compose.relay.yml up -d --build`. O exemplo roda como usuário sem privilégios, com sistema de arquivos somente leitura, sem capabilities, e publica a porta só em `127.0.0.1` para ficar atrás do proxy com TLS. O relay encerra de forma limpa com `SIGTERM`.

### 6.5 Dokploy

1. **Create Service → Application**, fonte GitHub (`nextestudios/Vellcast`, branch `main`).
2. Build type **Dockerfile**: `infrastructure/docker/relay.Dockerfile`, contexto `.`.
3. **Environment**: `VELLCAST_RELAY_SECRET` (gerado com `openssl rand -hex 32`, guardado só no Dokploy), `VELLCAST_CREATE_ROOM_KEY`, `VELLCAST_TRUST_PROXY=true`.
4. **Domains**: domínio do relay, container port `8788`, HTTPS com Let's Encrypt. O Traefik do Dokploy termina o TLS e encaminha o upgrade de WebSocket.
5. Health check em `/healthz`.
6. Uma réplica só: o estado das salas fica em memória (ver `ARCHITECTURE.md` §5). Escalar horizontalmente exige roteamento por sala, que ainda não existe.

### 6.6 Proxy reverso

Mesmas regras da seção 5, apontando para a porta `8788`. Pontos específicos:

- `proxy_read_timeout` acima de 30 s (timeout de inatividade do relay).
- Sem buffering e sem compressão no caminho `/ws`.
- `client_max_body_size` pequeno para `/api` (os corpos têm poucos bytes); o WebSocket aceita mensagens de até 4 MiB.
- Com `VELLCAST_TRUST_PROXY=true`, o proxy precisa **sobrescrever** `X-Forwarded-For` (Nginx: `proxy_set_header X-Forwarded-For $remote_addr;`), senão o cliente pode forjar o IP usado no rate limit.

### 6.7 Capacidade

Medição local (Apple M1, loopback, build Release): 1 transmissor 1080p60 a ~8 Mbps para 20 espectadores usou ~5% de um núcleo e < 10 MB de RAM no relay. O limite prático é a **banda de saída** (bitrate × espectadores: 20 × 8 Mbps = 160 Mbps), não a CPU. Dimensionamento e custo em `MONETIZATION.md`.

### 6.8 Player web e links de convite (S-011)

O relay também serve o player web (build de `apps/discord-activity`), para quem recebe um link de convite assistir no navegador sem instalar nada:

- `GET /` e `GET /watch` devolvem o `index.html`; os assets ficam em `/assets/*`. As rotas de API, `/ws`, `/healthz` e `/metrics` têm prioridade.
- O link gerado pelo desktop é `https://<relay>/watch#room=<id>&invite=<código>`. O código vai no **fragmento** (`#`), que o navegador nunca envia ao servidor; o player troca o código por um token de espectador (`POST /api/rooms/{id}/join`), guarda o token só na aba (`sessionStorage`) e remove o código da barra de endereços.
- Cabeçalhos: `Content-Security-Policy` restrita à própria origem (sem scripts inline), `X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer` e `Cache-Control: no-cache` no HTML.
- Para servir a partir do repositório: `VELLCAST_STATIC_DIR=apps/discord-activity/dist` (após `npm run build`).

**HTTPS é obrigatório para quem assiste em outro aparelho.** O WebCodecs (decodificação H.264/Opus no navegador) só existe em contexto seguro: HTTPS ou `localhost`. Um link `http://192.168.x.x:8788` abre a página, mas não toca vídeo. Opções:

| Cenário | Como |
|---|---|
| Teste no mesmo PC | `http://127.0.0.1:8788` (localhost é contexto seguro). |
| Teste rápido com outros aparelhos | Túnel HTTPS: `cloudflared tunnel --url http://localhost:8788`. No desktop, deixe o servidor em `http://127.0.0.1:8788` e preencha **Configurações → Servidor → Endereço público para os links de convite** com o `https://….trycloudflare.com`. Com túnel, todos os espectadores chegam pelo mesmo IP local; o rate limit por IP passa a ser compartilhado. |
| Uso contínuo | VPS com Dokploy e TLS (§6.5). No desktop, use `https://seu-dominio` como servidor; o app transmite por `wss://` automaticamente. |

### 6.9 Pacote de teste para Windows

O workflow `.github/workflows/windows-mvp.yml` gera o artefato `Vellcast-MVP-windows-x64` (desktop, relay, player, `iniciar-relay.bat`, `LEIA-ME.txt` e avisos de licença) a cada push na `main`. O `iniciar-relay.bat` gera um segredo aleatório em `relay.env` na primeira execução, escuta em `0.0.0.0:8788` e serve o player da pasta `player`. Passo a passo em `docs/TESTING_WINDOWS.md`.

## 7. Futuro: QUIC (Etapa 6)

- **QUIC (Quinn)**, se adotado entre desktop e relay, usa **UDP** (ex.: 443/udp ou porta dedicada):
  - Liberar a porta UDP no firewall da VPS e no provedor.
  - **Não presumir que o proxy reverso encaminha QUIC.** O Traefik/Nginx do Dokploy normalmente termina HTTP(S)/TCP; o QUIC do relay provavelmente precisará de porta UDP publicada diretamente no container (`ports: "4433:4433/udp"`) com certificado TLS próprio montado no container.
  - A Activity continua usando WebSocket via proxy do Discord.
- Monitoramento: logs estruturados (JSON), `/healthz`, métricas agregadas (S-014).
