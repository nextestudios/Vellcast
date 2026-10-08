# Captura de áudio (`crates/audio`)

Status: **implementado; compilado para Windows, sem execução real em Windows** (ETAPA 4, M-009). Decisões: ADR-023, ADR-024 e ADR-025 em [`DECISIONS.md`](../DECISIONS.md).

## API

```rust
let capture = AudioCapture::start(
    AudioTarget::Process(pid),           // ou AudioTarget::System
    origin_qpc,                          // mesma origem QPC do vídeo
    Box::new(|p: AudioPacket| { /* p.data = Opus 20 ms, p.pts_us */ }),
    Box::new(|aviso: String| { /* mostrar ao usuário */ }),
)?;
capture.set_volume(1.0);
let stats = capture.stats();             // pacotes entregues, ativo?
capture.stop();
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

## Limitações conhecidas

- O loopback por processo exclui só **uma** árvore: com dois clientes Discord abertos, um pode ser capturado (há aviso).
- No fallback clássico não é possível excluir o Discord.
- A deriva entre o relógio do dispositivo e o QPC é corrigida em degraus de ≥ 10 ms (pequeno corte ou silêncio audível em sessões longas, a medir na ETAPA 6).
- O atraso algorítmico do Opus (~6,5 ms em modo Audio) não é descontado do PTS.
- Não testado em hardware real: tudo o que depende de WASAPI foi apenas compilado (`cargo check`/`clippy` para `x86_64-pc-windows-gnu`, sem a libopus). A compilação completa com a libopus depende da CI Windows.

## Testes

| Teste | Onde roda | Cobre |
|---|---|---|
| `frames::tests` (9) | qualquer SO | blocos de 20 ms, jitter, lacunas, sobreposição, reinício, monotonia |
| `convert::tests` (8) | qualquer SO | PCM 16/24/32 bits, float, mono, downmix 5.1, ganho, reamostragem 44,1 → 48 kHz |
| `process::tests` (5) | qualquer SO | raiz da árvore do Discord, múltiplos clientes, sem Discord |
| `tests/capture_windows.rs` (2, `#[ignore]`) | Windows com saída de áudio | captura real do sistema por 1 s; loopback por processo |
| `examples/capture_audio.rs` | Windows | grava `.opus` para ouvir e imprime estatísticas de PTS |

Teste manual no Windows:

```powershell
cargo test -p vellcast-audio -- --ignored
cargo run -p vellcast-audio --example capture_audio -- system 10 sistema.opus
cargo run -p vellcast-audio --example capture_audio -- <PID do jogo> 10 jogo.opus
```

Critérios: o arquivo toca no VLC sem cortes; com o Discord em chamada, a voz **não** aparece no modo sistema; "intervalos ≠ 20 ms" deve ser 0.
