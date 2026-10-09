# Arquitetura do Vellcast

Status: **Proposta**. Ela será refinada a cada etapa, e as decisões ficam registradas em [DECISIONS.md](DECISIONS.md).

## 1. Visão geral

A arquitetura é cliente-servidor, no modelo de um servidor dedicado como no TeamSpeak. O transmissor envia **uma única** transmissão codificada ao relay. O relay distribui a mesma sequência de bytes para todos os espectadores autorizados, **sem decodificar nem recodificar** (W-006).

```
┌──────────────────────────── Windows (transmissor) ────────────────────────────┐
│                                                                                │
│  ┌──────────┐   eventos   ┌──────────────────── Motor de transmissão ───────┐  │
│  │ UI Slint │◄───────────►│                                                 │  │
│  │ (thread  │  canais     │  capture ──► encoder ──► mux ──► transport ─────┼──┼──► WSS (QUIC: avaliar)
│  │ própria) │  limitados  │  (WGC/D3D11)  (MF/HW H.264)  ▲                  │  │
│  └────▲─────┘             │  audio (WASAPI) ──► Opus ────┘                  │  │
│       │                   │  metrics ◄── todos os estágios                  │  │
│  ┌────┴──────┐            └─────────────────────────────────────────────────┘  │
│  │controller │ (GilRs / XInput — somente quando a janela tem foco)             │
│  └───────────┘                                                                 │
└────────────────────────────────────────────────────────────────────────────────┘
                                        │
                                        ▼
┌────────────────────────── vellcast-relay (Linux/Docker) ───────────────────────┐
│  Axum: /ws (mídia) · /health · /metrics · estáticos da Activity               │
│  Room ─► Stream ─► broadcast limitado ─► fila por espectador ─► WS espectador │
│  Auth/tokens · rate limit · limites de sala/espectador/banda                   │
└────────────────────────────────────────────────────────────────────────────────┘
                                        │ WebSocket (via proxy do Discord)
                                        ▼
┌──────────────── Discord Activity (React + TS + Vite, iframe) ──────────────────┐
│  Embedded App SDK · WS client · VideoDecoder/AudioDecoder (WebCodecs)         │
│  canvas + AudioContext · métricas · fallback teclado/mouse · Gamepad API (?)  │
└────────────────────────────────────────────────────────────────────────────────┘
```

## 2. Componentes e fronteiras

| Componente | Local | Responsabilidade | Depende de |
|---|---|---|---|
| `vellcast-protocol` | `crates/protocol` | Cabeçalho binário v1, tipos de mensagem, limites e (de)serialização. Sem I/O. | — |
| `capture` | `crates/capture` | Windows Graphics Capture (janela/monitor), DXGI como alternativa, texturas D3D11 | windows-rs |
| `encoder` | `crates/encoder` | H.264 por hardware (Media Foundation; NVENC/AMF/QSV quando justificado), keyframes, reconfiguração | capture (texturas) |
| `audio` | `crates/audio` | WASAPI (Application Loopback por processo, loopback de sistema como alternativa), Opus | — |
| `controller` | `crates/controller` | GilRs/XInput, mapeamento, deadzone, repetição, eventos de navegação | — |
| `transport` | `crates/transport` | Cliente WSS (inicial), QUIC (avaliar), heartbeats, reconexão, backpressure | protocol |
| `metrics` | `crates/metrics` | Contadores e histogramas locais e baratos, sem alocação no caminho quente | — |
| `auth` | `crates/auth` | Tokens temporários, pareamento, validação | — |
| `vellcast-core` | `crates/vellcast-core` | Orquestra o pipeline e a máquina de estados da sessão | crates acima |
| Desktop | `apps/desktop` | UI Slint, bandeja, teclado virtual | vellcast-core, controller |
| Activity | `apps/discord-activity` | Player WebCodecs dentro do Discord | protocolo (espelhado em TS) |
| Relay | `services/relay` | Salas, autorização, fan-out, limites, métricas | protocol, auth |
| Cloud API | `services/cloud-api` | Contas, planos e entitlements. Começa só com contratos. | — |
| Servidor ETAPA 0 | `services/stage0-server` | **Descartável.** Serve mídia sintética pré-codificada para validar a Activity. | protocol |

Estado atual do workspace Cargo: somente `crates/protocol` e `services/stage0-server` são membros. Os demais diretórios têm apenas um README de escopo. Nenhum crate vazio é compilado.

## 3. Fluxo de dados

### 3.1 Vídeo
1. **Captura** (`crates/capture`, ADR-018). O WGC entrega um `ID3D11Texture2D` (BGRA) por quadro novo, de um pool *free-threaded* com 2 buffers. O quadro **fica na GPU**: o `Direct3D11CaptureFrame` permanece vivo até o encoder ler a textura. O callback aplica o controle de ritmo (taxa-alvo) e coloca o quadro em uma fila de capacidade 1 (ADR-022).
2. **Conversão** (`crates/encoder/src/convert.rs`, ADR-019). `ID3D11VideoProcessor` converte BGRA → NV12 e redimensiona em um rodízio de 4 texturas NV12 de tamanho fixo; mudanças de tamanho da janela viram letterbox.
3. **Codificação** (`crates/encoder/src/mf.rs`, ADR-020). O MFT H.264 (hardware primeiro) recebe a superfície DXGI via `IMFDXGIDeviceManager` (sem cópia para a CPU) e produz uma unidade de acesso em Annex B. Baixa latência, CBR, sem B-frames, GOP de 2 s, IDR sob demanda. A saída é normalizada (`annexb::normalize`): start codes de 4 bytes, sem AUD, SPS/PPS antes de todo IDR; o codec string `avc1.PPCCLL` sai do SPS.
4. **Empacotamento.** `VideoFrame` (tipo 0x02) com `pts_us` do relógio de captura e `sent_at_us` do relógio de parede. SPS/PPS vão repetidos antes de cada IDR.
5. **Transporte.** Fila de envio limitada. Sob congestionamento, frames não-keyframe antigos são descartados e uma IDR é pedida ao encoder.

### 3.2 Áudio
WASAPI (loopback por processo quando o Windows suportar, ou loopback do sistema) → PCM float 48 kHz → Opus em pacotes de 20 ms → `AudioFrame` (0x04) com `pts_us` no **mesmo relógio** do vídeo.

### 3.3 Inicialização de um novo espectador
1. O espectador conecta e se autentica. O relay envia `VideoInit`/`AudioInit` (JSON com codec, resolução, fps, sample rate).
2. O relay só começa a entregar vídeo a partir do **próximo keyframe**. Se a espera passar do limite, o relay envia `KeyframeRequest` ao transmissor.
3. O cliente configura o `VideoDecoder` com o codec string `avc1.PPCCLL` derivado do SPS e sem `description` (formato Annex B).

Formato detalhado: [STREAMING_PROTOCOL.md](STREAMING_PROTOCOL.md).

### 3.4 Relógio e latência
O cliente mede a diferença de relógio com Ping/Pong no estilo NTP: `offset = t_servidor − (t0 + t1)/2`, usando a amostra de menor RTT. Na ETAPA 0, **latência = envio pelo servidor → frame decodificado no cliente**. Captura e codificação não entram nessa conta. A latência ponta a ponta real (captura → tela) será medida na ETAPA 5.

## 4. Modelo de threads do desktop

```
Thread UI (Slint event loop) ── só desenha e reage a eventos; sem I/O, sem espera
     ▲  slint::invoke_from_event_loop (estado resumido, ≤ 4 Hz para métricas)
     │
Pool WGC (free-threaded, ritmo) ─► fila "o mais recente vence" (cap. 1) ─► Thread vellcast-pipeline
                                   substitui o pendente                  (D3D11 + conversão + MFT)
Thread de áudio (WASAPI evento) ─► Opus ─────────────────────────────┐            ▼
                                                                    ├─► Runtime Tokio (transporte, 1–2 workers)
Thread do controle (GilRs, só com foco) ─► eventos de navegação ─► UI
```

Regras:
- A UI nunca bloqueia em captura, encoder ou rede. A comunicação com o motor é por canais limitados e comandos assíncronos.
- Com a janela minimizada ou oculta, a prévia é desligada e não há redraw contínuo. Métricas só chegam à UI quando a tela de métricas está visível.
- Nenhuma thread do motor eleva a prioridade a ponto de competir com o jogo. O Modo Competitivo reduz a carga (bitrate → FPS → resolução) com histerese.
- O controle só é lido com a janela em foco. Atalhos globais são opcionais e vêm desligados.

### 4.1 Adaptação de qualidade (S-001, S-002)

Uma vez por segundo o pipeline entrega ao controlador (`crates/vellcast-core/src/adapt.rs`) os sinais da última janela: bytes na fila do transporte, quadros descartados pela rede, quadros recusados pelo encoder, RTT e tempo médio de codificação. O controlador devolve um bitrate e um FPS-alvo:

| Situação | Equilibrado | Competitivo |
|---|---|---|
| Rede congestionada | bitrate ×0,7 (≥ 2 s entre reduções) | bitrate ×0,7 (≥ 1 s entre reduções) |
| Encoder sobrecarregado | após 3 s: FPS 60 → 45 → 30 | na hora: bitrate ×0,8 até 60%, depois FPS até 24, depois recomenda resolução menor |
| Estável | +5% do bitrate após 5 s; FPS volta após 15 s | +5% após 10 s; FPS volta após 30 s |

O bitrate é aplicado no encoder em tempo real (`CODECAPI_AVEncCommonMeanBitRate`); o FPS reduzido é obtido descartando quadros antes do encoder (o mais recente fica guardado para repetição). Reduções geram `SessionEvent::Warning` (no máximo um a cada 15 s); `SessionStats` expõe `target_bitrate_kbps` e `target_fps`, mostrados em "Métricas avançadas".

## 5. Internos do relay

Implementado em `services/relay` (ETAPA 3). Módulos: `api` (HTTP), `ws` (sessões), `room` (estado de mídia), `store` (metadados), `token`, `ratelimit`, `metrics`, `config`.

```
Relay
 ├─ RoomStore (trait)  ← metadados: id, nome, dono, TTL, limite de espectadores, convites (hash)
 │    └─ MemoryStore   (hoje; SQLite/PostgreSQL implementam o mesmo trait no futuro)
 └─ LiveRoom (sempre em memória, uma por sala)
      ├─ Mutex<Inner>: estado (idle/live/paused/closed), vaga do transmissor,
      │                VideoInit, AudioInit, GopCache (≤ 8 MiB)
      ├─ broadcast::Sender<Relayed>  (capacidade = fila por espectador)
      ├─ Publisher: 1 tarefa WS → valida cabeçalho, tamanho, banda e JSON de init
      │             → publish() (atualiza cache + broadcast sob o mesmo lock)
      └─ Viewers: N tarefas WS → snapshot (init + GOP) → broadcast::Receiver
                  → descarta quadros > 500 ms / Lagged → espera keyframe e pede um ao transmissor
```

- **Zero cópia e zero recodificação.** Cada mensagem do transmissor vira um `Bytes` com contagem de referência e vai igual para todos os espectadores. O relay não reescreve nem o cabeçalho; os testes de integração comparam os bytes recebidos por cada espectador com os enviados.
- **Isolamento.** Cada espectador tem tarefa e fila próprias. Um espectador lento perde mensagens antigas (`Lagged`) e volta no próximo keyframe; se o envio ficar travado por 10 s, é desconectado. **Ele nunca bloqueia o transmissor nem os outros espectadores** (há teste de integração para isso).
- **Entrada imediata.** O GOP atual em cache permite que um espectador novo comece a decodificar na hora, sem esperar o próximo keyframe.
- **Queda do transmissor.** Sem `StreamEnd`, a sala vai para `paused` e avisa os espectadores (`StreamPaused`). Se o transmissor voltar dentro da tolerância (30 s), a transmissão continua com `VideoInit` + `DISCONTINUITY`; senão, `StreamEnd`.
- **Limpeza.** Uma tarefa periódica remove salas com TTL vencido, salas ociosas, convites expirados e buckets de rate limit inativos.
- **Fronteira de persistência.** Só os metadados passam por `RoomStore`. O caminho de mídia nunca toca disco nem banco, o que mantém a latência independente da persistência.
- **Escala.** Uma instância guarda as salas em memória; várias instâncias exigiriam roteamento por sala (fora do escopo atual). O gargalo esperado é a banda de saída, não a CPU (ver `PERFORMANCE.md`).
- **Heartbeats.** Clientes enviam `Ping`; 30 s sem mensagens → `idle_timeout`.
- **Sem estado de mídia persistente.** Nada é gravado em disco (W-008).

## 6. Discord Activity e rede

- A Activity é servida no mesmo domínio do relay, via URL Mapping `/` → host do relay. Para o código, o WebSocket fica em `wss://<client_id>.discordsays.com/ws`.
- A documentação oficial de networking cita **somente WebSocket** como transporte suportado. WebRTC não é suportado e WebTransport ainda não. Por isso o protocolo é desenhado sobre mensagens binárias de WebSocket.
- `patchUrlMappings` só será usado se houver hosts adicionais. A CSP bloqueia hosts não mapeados (`blocked:csp`).
- A identidade vem do OAuth do Discord, trocada **no backend**. O frontend nunca é fonte de verdade.

## 7. Tratamento de falhas

| Falha | Comportamento |
|---|---|
| Janela capturada fechada | O WGC sinaliza `Closed`; o motor encerra a transmissão com `StreamEnd` e a UI mostra o motivo |
| Mudança de resolução da fonte | Recria o frame pool; o encoder mantém o tamanho de saída e o conteúdo é encaixado com letterbox (ADR-019) |
| Perda de dispositivo D3D (`DXGI_ERROR_DEVICE_REMOVED`/`RESET`) | Para a captura, descarta quadros pendentes, recria dispositivo, captura e encoder (até 3 vezes, com aviso na UI) e pede IDR; depois encerra com motivo visível |
| Falha do encoder de hardware | Na criação, tenta o próximo encoder detectado. Software só como último recurso e com aviso explícito, por causa do impacto no jogo |
| Rede congestionada | Descarta frames antigos, pede IDR e reduz bitrate (S-001) |
| Desconexão do transmissor | Relay marca o stream como interrompido e mantém a sala por uma janela de graça; o cliente reconecta com backoff exponencial |
| Desconexão do espectador | Reconecta com backoff e retoma no próximo keyframe |
| Gamepad desconectado | Mantém o estado de foco e retoma a navegação quando reconectar (S-010) |
| Decoder do cliente em erro | Reseta o `VideoDecoder`, aguarda keyframe e pede `KeyframeRequest` |

## 8. Implantação

- Relay: um binário estático em uma imagem Docker mínima, atrás de um proxy reverso com TLS (Dokploy/Traefik). Healthcheck em `/health`.
- QUIC (se adotado na S-005) exige porta UDP exposta diretamente. Nem todo proxy encaminha QUIC.
- Banco: nenhum na ETAPA 0. SQLite para self-hosting e PostgreSQL avaliado para a Cloud. Sem Redis até haver necessidade demonstrada.

Detalhes em [DEPLOYMENT.md](DEPLOYMENT.md).
