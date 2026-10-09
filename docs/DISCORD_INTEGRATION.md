# Integração com Discord Activities

Status: **Activity com autenticação OAuth2 (backend), pareamento e player implementados e testados com API do Discord simulada. Validação dentro de uma Activity real: Blocked** — requer o responsável criar a aplicação no Discord Developer Portal (seção 4). A URL HTTPS pública já existe: `https://vellcast.nextestudios.com`.

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

## 3. Embedded App SDK, autenticação e pareamento (M-014, M-015)

- Pacote: `@discord/embedded-app-sdk` **2.5.0** (MIT), carregado só dentro do Discord (import dinâmico).
- Dentro do Discord (URL com `frame_id`) a página abre o `ActivityApp`; fora dele, `/watch` (convite) e a página de validação continuam iguais. `?stage0` força a página de validação dentro do Discord.

### 3.1 Autenticação (OAuth2 no backend)

1. `sdk.ready()` → `sdk.commands.authorize({ client_id, response_type: 'code', state: '', prompt: 'none', scope: ['identify'] })` (escopo mínimo).
2. A Activity envia o `code` para `POST /api/discord/token` (mesma origem, via URL Mapping).
3. O relay troca o código em `https://discord.com/api/v10/oauth2/token` com o **client secret, que só existe no servidor**, consulta `GET /users/@me` com o access token e emite uma **sessão Vellcast** (HMAC, 2 h) vinculada ao ID do usuário. A identidade nunca vem do frontend.
4. A Activity chama `sdk.commands.authenticate({ access_token })`.

### 3.2 Pareamento desktop ↔ Activity

1. No app desktop: **Salas → selecionar a sala → Parear com o Discord**. O relay (`POST /api/rooms/{id}/pairing`, só com token de dono) gera um código de 8 caracteres (sem 0/O/1/I/L), válido por 5 minutos e de uso único.
2. Na Activity, alguém digita o código (teclado físico ou teclado na tela, navegável por controle) → `POST /api/discord/pair` com a sessão e o `instanceId` da Activity.
3. O relay vincula a instância à sala (até expirar a sala) e todos os participantes dessa instância passam a ver a transmissão em `GET /api/discord/instances/{instanceId}/streams`, que devolve tokens de espectador novos a cada consulta.
4. Proteções: rate limit estrito por IP nas trocas de código e tentativas de pareamento, código de uso único, limite de 3 códigos ativos por sala e 8 salas por instância, `instanceId` validado. Com `VELLCAST_DISCORD_BOT_TOKEN` definido, o relay também confirma pela API do Discord que o usuário está na instância (`GET /applications/{app}/activity-instances/{instance}`); sem ele, quem conhece o `instanceId` e tem uma sessão Discord poderia listar as salas daquela instância (risco aceito no MVP, documentado em `SECURITY.md`).

### 3.3 Configuração do servidor

| Variável (Dokploy → Environment) | Obrigatória | Uso |
|---|---|---|
| `VELLCAST_DISCORD_CLIENT_ID` | sim | ID da aplicação (público) |
| `VELLCAST_DISCORD_CLIENT_SECRET` | sim | segredo OAuth2 — **nunca** no repositório nem no build da Activity |
| `VELLCAST_DISCORD_BOT_TOKEN` | não | confirma que o usuário participa da instância |
| `VELLCAST_DISCORD_API_BASE` | não | só para testes (API simulada) |

Sem client ID/secret as rotas `/api/discord/*` respondem 503 e o restante do relay funciona normalmente.

## 4. Passo a passo para validar no Discord (E0-T10)

**Bloqueado até o responsável criar a aplicação no Discord.** Depois disso, cerca de 15 minutos:

1. https://discord.com/developers/applications → **New Application** (ex.: "Vellcast").
2. **Activities → Settings**: habilitar Activities.
3. **Activities → URL Mappings**: prefixo `/` → `vellcast.nextestudios.com` (sem `https://`).
4. **OAuth2 → Redirects**: adicionar `https://127.0.0.1` (exigido pelo portal; o SDK cuida do retorno). Copiar **Client ID** e **Client Secret**.
5. (Opcional) **Bot**: gerar o token, para a checagem de participação na instância.
6. Dokploy → aplicação do relay → **Environment**: definir as variáveis da tabela 3.3.
7. Build do player com o client ID (público): no Dokploy, **Build Args** `VITE_DISCORD_CLIENT_ID=<id>` (o `infrastructure/docker/relay.Dockerfile` já repassa o arg ao build do player). **Redeploy**.
8. No Discord Desktop: entrar em um canal de voz → **Activities** → Vellcast (enquanto a aplicação não é pública, só membros da equipe do app veem a Activity; adicionar testadores em **App Testers**).
9. No app desktop: criar sala → **Parear com o Discord** → digitar o código na Activity → iniciar a transmissão.
10. Repetir em Discord Web e Mobile e preencher a matriz abaixo.

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
