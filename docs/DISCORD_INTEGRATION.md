# Integração com Discord Activities

Status: **Activity mínima implementada e testada fora do Discord. Validação dentro de uma Activity real: Blocked** (requer aplicação no Discord Developer Portal e URL HTTPS pública).

Documentação oficial:
- Visão geral: https://docs.discord.com/developers/activities/overview
- Networking: https://docs.discord.com/developers/activities/development-guides/networking

## 1. Restrições de rede (consultadas em 2026-10-08)

- Todo tráfego da Activity passa pelo **proxy do Discord**; a página roda em `https://<client_id>.discordsays.com`.
- Hosts externos sem URL Mapping são bloqueados por CSP (`blocked:csp`).
- Transporte em tempo real: **apenas WebSocket**. WebRTC não é suportado; WebTransport ainda não é suportado (o Discord diz estar trabalhando nisso com os provedores upstream).
- A documentação **não informa** limites de tamanho de mensagem nem de throughput no proxy. Por isso, 1080p60 a ~8 Mbps dentro do Discord só pode ser confirmado com teste real (risco R-03 em `RISKS.md`).

Consequência para a arquitetura: H.264/Opus encapsulados no protocolo próprio (`STREAMING_PROTOCOL.md`) sobre WebSocket e decodificados com WebCodecs no cliente. Ver ADRs em `DECISIONS.md`.

## 2. Como a Activity se conecta

A Activity e o servidor estão na **mesma origem**:

- URL Mapping no Developer Portal: prefixo `/` → host do servidor (ex.: `stage0.seudominio.com`).
- A página usa caminhos relativos: `fetch('/api/streams')` e `wss://<origem>/ws?stream=...`.
- Assim, não é preciso usar `patchUrlMappings` (que altera `fetch`/`WebSocket` globais). Se algum dia houver mais de um backend, o mapeamento passa a ser por prefixo (ex.: `/relay` → host do relay).

## 3. Embedded App SDK

- Pacote: `@discord/embedded-app-sdk` **2.5.0** (MIT).
- Detecção: a página está dentro do Discord quando a URL contém `frame_id`. Fora disso, roda em **modo navegador** (útil para testes locais) e o SDK nem é carregado (import dinâmico).
- Na ETAPA 0 só se chama `new DiscordSDK(clientId)` + `await sdk.ready()`, e a UI mostra `platform`, `instanceId`, `guildId` e `channelId`.
- **OAuth2 (`authorize` → troca do código no backend → `authenticate`) fica para a ETAPA 5.** A troca exige o `client_secret`, que só pode existir no servidor; nada de identidade vindo do frontend é confiável.

## 4. Passo a passo para validar no Discord (E0-T10)

Pré-requisitos que dependem do responsável pelo projeto:

1. Criar uma aplicação em https://discord.com/developers/applications e habilitar **Activities**.
2. Obter uma URL HTTPS pública para o servidor. Opções:
   - túnel temporário: `cloudflared tunnel --url http://localhost:8787`;
   - ou deploy via Dokploy (ver `DEPLOYMENT.md`).
3. Em **Activities → URL Mappings**: `/` → host da etapa 2 (sem `https://`).
4. Build da Activity com o client ID (é um ID público, não um segredo):
   ```bash
   cd apps/discord-activity
   echo "VITE_DISCORD_CLIENT_ID=<id>" > .env
   npm run build
   ```
5. Rodar o servidor (`cargo run --release -p vellcast-stage0-server`) e abrir a Activity em um canal de voz.
6. Para cada cliente (Desktop, Web, Mobile): selecionar `720p30` e `1080p60`, ativar o áudio, aguardar 60 s e usar **Copiar relatório**. Registrar os resultados em `PERFORMANCE.md`.

## 5. Matriz de validação

| Item | Navegador local (Chromium headless) | Discord Desktop | Discord Web | Discord Mobile |
|---|---|---|---|---|
| `sdk.ready()` | n/a (modo navegador) | Pendente | Pendente | Pendente |
| `VideoDecoder` H.264 `avc1.64002A` | Suportado | Pendente | Pendente | Pendente |
| 720p30 reproduzindo | OK (30/30 fps) | Pendente | Pendente | Pendente |
| 1080p60 reproduzindo | OK (60/60 fps) | Pendente | Pendente | Pendente |
| `AudioDecoder` Opus | Suportado | Pendente | Pendente | Pendente |
| Reconexão automática | OK | Pendente | Pendente | Pendente |
| Gamepad API | API presente, policy permite; sem controle físico | Pendente | Pendente | Pendente |
| Tela cheia | Não testado | Pendente | Pendente | Pendente |

Detalhes e números em `PERFORMANCE.md`.

## 6. Pontos a investigar no teste real

- Se o iframe do Discord concede `gamepad` na Permissions Policy (a UI mostra o resultado).
- Se `requestFullscreen` é permitido no iframe (a UI informa quando é bloqueado).
- Se o cliente mobile (WebView) expõe `VideoDecoder`/`AudioDecoder`. Caso o `AudioDecoder` não exista, avaliar decodificação Opus em WASM antes de considerar o áudio concluído.
- Latência e estabilidade do proxy do Discord a ~8 Mbps.
- Comportamento do player quando a Activity é minimizada ou vira PiP (a UI deixa de atualizar métricas com `document.hidden`).
