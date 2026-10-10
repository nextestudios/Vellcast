# Captura de áudio (`crates/audio`)

Status: **implementado; compilado para Windows, sem execução real em Windows** (ETAPA 4, M-009). Decisões: ADR-023, ADR-024 e ADR-025; microfone e mudanças ao vivo: ADR-062 (issue #5) em [`DECISIONS.md`](../DECISIONS.md).

## API

```rust
let capture = AudioCapture::start(
    AudioTarget::Process(pid),           // ou AudioTarget::System
    origin_qpc,                          // mesma origem QPC do vídeo
    Box::new(|p: AudioPacket| { /* p.data = Opus 20 ms, p.pts_us */ }),
    Box::new(|aviso: String| { /* mostrar ao usuário */ }),
)?;
capture.set_volume(1.0);
let stats = capture.stats();             // pacotes entregues, ativo?, loopback?, microfone
capture.stop();

// Microfone (opt-in) e mudanças durante a transmissão, sem bloquear:
let capture = AudioCapture::start_with(
    AudioSetup { loopback: None, microphone: Some(MicConfig::default()) },
    origin_qpc, on_packet, on_warning,
)?;                                      // sem loopback: retorna na hora
capture.set_loopback(Some(AudioTarget::System)); // liga/troca/desliga (None) o áudio do jogo
capture.set_microphone(Some(MicConfig { device_id: Some(id), muted: false }));
capture.set_mic_muted(true);
let mics = vellcast_audio::list_input_devices()?; // vazia sem microfone
```

- `start` só retorna depois que o dispositivo foi aberto (ou com o erro), em até 5 s.
- Os callbacks rodam na thread `vellcast-audio`, nunca na thread da UI.
- Fora do Windows, `start` retorna `AudioError::Unsupported`.

## Fluxo

```
WASAPI (evento) ─► GetBuffer + qpcPosition
                    │
                    ├─ formato nativo 48 kHz estéreo f32 (conversão do Windows)
                    └─ ou Converter: downmix + reamostragem linear (fallback)
                    │
                    ▼
            FrameAssembler: blocos de 960 amostras (20 ms), PTS contíguos
                    │   lacunas → silêncio · sobreposição → corte · jitter ≤ 10 ms ignorado
                    ▼
            Opus 128 kbps (libopus) ─► on_packet(AudioPacket)
```

| Alvo | Mecanismo | Requisito | Fallback |
|---|---|---|---|
| `Process(pid)` | Loopback **incluindo** a árvore do processo da janela | Windows 10 2004+ (build 19041) | Erro `ProcessLoopbackUnavailable` (o núcleo decide e avisa) |
| `System` | Loopback **excluindo** a árvore do Discord (ou o próprio Vellcast, sem Discord) | Windows 10 2004+ | Loopback clássico do dispositivo padrão + aviso de que a voz do Discord pode ser capturada |

## Robustez

- **Sem polling ativo:** a thread espera o evento do WASAPI com timeout de 20 ms (usado também para checar a parada e completar silêncio).
- **Memória limitada:** buffers são reutilizados; o montador nunca guarda mais de um bloco parcial.
- **Silêncio:** o loopback clássico não entrega pacotes quando nada toca. A linha do tempo é completada com silêncio até `agora − 100 ms`, mantendo o áudio contínuo e alinhado.
- **Dispositivo:** `AUDCLNT_E_DEVICE_INVALIDATED` ou outra falha reabre a captura com backoff (200 ms → 2 s). No loopback clássico, a troca do dispositivo padrão é detectada a cada 1 s. O loopback por processo segue o dispositivo padrão sozinho.
- **Volume:** ganho linear de 0 a 2, aplicado antes da codificação, com limite em ±1.0.

## Microfone e mudanças durante a transmissão (issue #5)

Os controles rápidos da tela Ao vivo mudam o áudio sem parar a transmissão. Tudo é aplicado pela thread `vellcast-audio` em até ~20 ms; quem chama (o pipeline) nunca espera dispositivo.

```
                     ┌─ loopback (app ou sistema) ─► FrameAssembler ─┐
vellcast-audio ──────┤   ou só silêncio (a cada 20 ms)              ├─► bloco 20 ms ─► + microfone ─► Opus ─► relay
                     └───────────────────────────────────────────────┘                    ▲
vellcast-mic ── WASAPI captura (evento) ─► 48 kHz estéreo f32 ─► MicFifo (300 ms máx.) ───┘
```

- **Áudio do jogo** (`set_loopback`): `None` desliga; `Process(pid)` ou `System` reabrem o loopback. Sem loopback, a linha do tempo continua com silêncio (folga de 40 ms), então o player não perde a sincronia quando o som volta. A troca para "só o aplicativo" que falhar cai para o modo sistema (com a exclusão do Discord) e avisa; se nada abrir, fica só com silêncio e avisa. O modo sistema mantém a regra de sempre: exclui a árvore do Discord ([`process.rs`](../../crates/audio/src/process.rs)).
- **Microfone** (`set_microphone`, **desligado por padrão**): uma thread `vellcast-mic` por dispositivo abre o endpoint de captura (`eCapture`; `None` = padrão do Windows) em modo compartilhado orientado a evento, pedindo 48 kHz estéreo `f32` ao Windows (`AUTOCONVERTPCM`); se o driver recusar, usa o formato de mixagem e o `Converter` (downmix de mono/multicanal e reamostragem linear). As amostras vão para a `MicFifo`.
- **Mixagem** ([`mix.rs`](../../crates/audio/src/mix.rs)): a cada bloco de 20 ms do loopback (ou do silêncio), a thread principal retira 960 amostras por canal da fila e soma ao bloco com um limitador suave (linear até ±0,8, compressão até ±1,0). A fila é limitada a 300 ms (descarta as mais antigas: relógio do microfone adiantado) e só entrega depois de 40 ms de pré-carga (falta de dados: o bloco sai sem microfone e a fila volta a pré-carregar). Latência típica do microfone em relação ao jogo: 40–60 ms.
- **Mudo** (`set_mic_muted`): o dispositivo continua aberto, mas os pacotes são descartados na própria thread do microfone e a fila é esvaziada: nada do microfone chega ao Opus.
- **Gerações:** cada troca de microfone incrementa uma geração; a thread antiga para em até 20 ms e não escreve mais na fila nem no estado. Threads encerradas são reunidas sem bloquear o áudio.
- **Desconexão:** `AUDCLNT_E_DEVICE_INVALIDATED` ou falha ao abrir reabre o microfone com backoff (200 ms → 2 s). Depois de 4 tentativas, o estado vira `Unavailable`, o aviso "Microfone indisponível… A transmissão continua sem o microfone" chega à interface e a transmissão segue com o áudio do jogo. Sem nenhum microfone (ex.: runner da CI), `list_input_devices` devolve lista vazia e ligar o microfone termina em `Unavailable`, sem erro fatal.
- **Estado:** `AudioStats` expõe `loopback`, `mic` (`Off`, `Starting`, `Active`, `Unavailable`) e `mic_muted`; o núcleo repassa como `SessionStats::game_audio` e `SessionStats::microphone`.

## Limitações conhecidas

- O loopback por processo exclui só **uma** árvore: com dois clientes Discord abertos, um pode ser capturado (há aviso).
- No fallback clássico não é possível excluir o Discord.
- A deriva entre o relógio do dispositivo e o QPC é corrigida em degraus de ≥ 10 ms (pequeno corte ou silêncio audível em sessões longas, a medir na ETAPA 6).
- O atraso algorítmico do Opus (~6,5 ms em modo Audio) não é descontado do PTS.
- O microfone é alinhado por contagem de amostras (fila com pré-carga), não por timestamp: a deriva entre o relógio do microfone e o do loopback é corrigida por descarte (fila cheia) ou por um bloco sem microfone (fila vazia).
- Não há ganho próprio do microfone nem supressão de ruído; o nível é o do Windows.
- Não testado em hardware real: tudo o que depende de WASAPI foi apenas compilado (`cargo check`/`clippy` para `x86_64-pc-windows-gnu`, sem a libopus). A compilação completa com a libopus depende da CI Windows.

## Testes

| Teste | Onde roda | Cobre |
|---|---|---|
| `frames::tests` (10) | qualquer SO | blocos de 20 ms, jitter, lacunas, sobreposição, reinício, monotonia, linha do tempo só de silêncio |
| `mix::tests` (9) | qualquer SO | fila do microfone (pré-carga, falta de dados, limite de latência), limitador suave, mixagem com ganho, microfone mono 44,1 kHz reamostrado e mixado |
| `convert::tests` (8) | qualquer SO | PCM 16/24/32 bits, float, mono, downmix 5.1, ganho, reamostragem 44,1 → 48 kHz |
| `process::tests` (5) | qualquer SO | raiz da árvore do Discord, múltiplos clientes, sem Discord |
| `tests/capture_windows.rs` (2, `#[ignore]`) | Windows com saída de áudio | captura real do sistema por 1 s; loopback por processo |
| `examples/capture_audio.rs` | Windows | grava `.opus` para ouvir e imprime estatísticas de PTS |

Teste manual no Windows:

```powershell
cargo test -p vellcast-audio -- --ignored
cargo run -p vellcast-audio --example capture_audio -- system 10 sistema.opus
cargo run -p vellcast-audio --example capture_audio -- <PID do jogo> 10 jogo.opus
cargo run -p vellcast-audio --example capture_audio -- devices
cargo run -p vellcast-audio --example capture_audio -- system+mic 10 mix.opus
```

Critérios: o arquivo toca no VLC sem cortes; com o Discord em chamada, a voz **não** aparece no modo sistema; "intervalos ≠ 20 ms" deve ser 0.
