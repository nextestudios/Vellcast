# Protocolo de Streaming Vellcast — v1

Status: **v1 implementado na ETAPA 0** (`crates/protocol`, espelhado em `apps/discord-activity/src/protocol.ts`).
Escopo atual: servidor → espectador. O sentido transmissor → relay reutilizará o mesmo cabeçalho na ETAPA 3.

## 1. Transporte

- Uma mensagem **WebSocket binária** = um pacote do protocolo. Mensagens de texto são ignoradas.
- Em produção, sempre sobre TLS (`wss://`). Na Discord Activity, o WebSocket é aberto na mesma origem (`/ws`) e passa pelo proxy do Discord (ver `DISCORD_INTEGRATION.md`).
- Endpoint da ETAPA 0: `GET /ws?stream=<nome>` (ex.: `1080p60`, `720p30`).

## 2. Cabeçalho (28 bytes, little-endian)

| Offset | Tamanho | Campo | Descrição |
|---:|---:|---|---|
| 0 | 1 | `version` | Versão do protocolo. Atual: `1`. Versão desconhecida → mensagem descartada. |
| 1 | 1 | `type` | Tipo de mensagem (seção 3). |
| 2 | 2 | `flags` | bit 0 `KEYFRAME`; bit 1 `DISCONTINUITY`; demais reservados (enviar 0, ignorar ao ler). |
| 4 | 4 | `stream_id` | Identificador da transmissão. |
| 8 | 4 | `sequence` | Contador por transmissão (com wrap). No `Pong`, ecoa a sequência do `Ping`. |
| 12 | 8 | `pts_us` | Timestamp de mídia em µs. Vídeo e áudio compartilham a mesma base de tempo. |
| 20 | 8 | `sent_at_us` | Relógio de parede do emissor (Unix, µs) no momento do envio. |
| 28 | — | `payload` | Conteúdo, conforme o tipo. |

Por que little-endian: é a ordem nativa de x86/ARM e de `DataView(..., true)`, sem custo de conversão nos dois lados.

## 3. Tipos de mensagem

| Código | Nome | Sentido | Payload |
|---|---|---|---|
| `0x01` | `VideoInit` | S → C | JSON: `{ codec, codedWidth, codedHeight, framerate }`. `codec` no formato WebCodecs (`avc1.PPCCLL`, derivado dos bytes 1–3 do SPS). |
| `0x02` | `VideoFrame` | S → C | Uma unidade de acesso H.264 completa em **Annex B** (start codes `00 00 00 01`). |
| `0x03` | `AudioInit` | S → C | JSON: `{ codec: "opus", sampleRate: 48000, numberOfChannels }`. |
| `0x04` | `AudioFrame` | S → C | Um pacote Opus (normalmente 20 ms). |
| `0x10` | `Ping` | C → S | 8 bytes: `f64` LE com o relógio do cliente (ms). |
| `0x11` | `Pong` | S → C | Eco do payload do `Ping`; `sent_at_us` = relógio do servidor. |
| `0x12` | `KeyframeRequest` | C → S | Vazio. Pede um keyframe ao transmissor. |
| `0x20` | `StreamEnd` | S → C | Vazio. A transmissão terminou. |

Tipos desconhecidos são rejeitados por quem lê (`DecodeError::UnknownType`); por isso, um tipo novo exige nova versão **ou** negociação explícita.

## 4. H.264

- Perfil usado nos testes: High (`avc1.64002A` em 1080p60, `avc1.64001F` em 720p30), **sem B-frames** (decodificação em ordem, menor latência), uma slice por quadro.
- Formato **Annex B**. O `VideoDecoder` é configurado **sem `description`**, o que, pelo registro AVC do WebCodecs, indica bitstream Annex B com parâmetros in-band.
- **SPS/PPS precedem todo IDR.** O servidor garante isso mesmo que a fonte não repita os parâmetros (`media::parse_h264` reinsere o último SPS/PPS). Assim, qualquer keyframe é um ponto de entrada válido.
- AUD (NAL 9) é usado só para delimitar quadros na leitura do arquivo e é removido antes do envio.
- Flag `KEYFRAME` = a unidade de acesso contém um IDR (NAL 5).

## 5. Inicialização de um novo espectador

1. O servidor inscreve o espectador na fila de distribuição **antes** de enviar a configuração (não perde quadros entre as duas etapas).
2. Envia `VideoInit` e `AudioInit` (cache da transmissão).
3. Descarta `VideoFrame` delta até o primeiro `KEYFRAME`. O áudio flui desde o início.
4. O cliente configura os decoders ao receber os `Init` e só decodifica a partir de um keyframe.

Com keyframe a cada 1 s (ETAPA 0), o tempo até o primeiro quadro é de no máximo ~1 s + RTT. A redução desse tempo (keyframe sob demanda via `KeyframeRequest`) depende do encoder real (ETAPA 2/3).

## 6. Timestamps e relógio

- `pts_us` começa em 0 e cresce continuamente; no loop da mídia de teste, o servidor soma a duração do loop para não haver descontinuidade.
- `DISCONTINUITY` sinaliza saltos (troca de fonte, reconfiguração do encoder). Ainda não é emitido na ETAPA 0.
- **Sincronização de relógio** (estilo NTP): o cliente envia `Ping(t0)`, recebe `Pong` com `sent_at_us = ts` em `t1`. `rtt = t1 − t0`; `offset = ts − (t0 + t1)/2`. Usa-se a amostra de menor RTT entre as 10 últimas.
- **Latência medida** = `agora_local − (sent_at_us − offset)` no momento em que o quadro sai do decoder. Mede servidor → quadro decodificado; **não** inclui captura, codificação nem uplink do transmissor.

## 7. Controle de fluxo e limites

| Limite | Valor (ETAPA 0) | Onde |
|---|---|---|
| Tamanho máximo de mensagem (S → C) | 4 MiB | `MAX_MESSAGE_LEN` |
| Tamanho máximo de mensagem (C → S) | 1 KiB | `MAX_CLIENT_MESSAGE_LEN` (frame e mensagem WebSocket) |
| Fila por espectador | 128 mensagens (~1 s) | `QUEUE_CAPACITY` (`tokio::sync::broadcast`) |
| Idade máxima de um quadro de vídeo ao sair da fila | 400 ms | `MAX_VIDEO_AGE_US` |
| Espectadores por transmissão | 50 | `MAX_VIEWERS_PER_STREAM` |
| Inatividade do cliente | 10 s sem mensagens → close `4000` | `CLIENT_IDLE_TIMEOUT` |
| Fila do decoder no cliente | > 4 quadros → descarta deltas até o próximo keyframe | `MAX_DECODE_QUEUE` |

Regras:

- Cada mensagem é serializada **uma vez** e compartilhada (`Bytes`, contagem de referência) entre todos os espectadores. Não há cópia nem recodificação por espectador.
- Um espectador lento nunca bloqueia os outros: ao estourar a fila, ele perde as mensagens mais antigas (`Lagged`) e volta a sincronizar no próximo keyframe.
- Quadros velhos (> 400 ms) são descartados no servidor em vez de aumentar a latência.

## 8. Versionamento

- O campo `version` muda quando o layout do cabeçalho ou a semântica de um tipo existente muda.
- Campos e flags reservados permitem extensões compatíveis dentro da v1.
- O cliente da Activity e o servidor são distribuídos juntos na ETAPA 0; a negociação de versão (`Hello` com versões suportadas) entra na ETAPA 3, quando transmissor e relay passam a ser versionados de forma independente.

## 9. Pendências conhecidas

- Autenticação e autorização por sala: ETAPA 3/5 (o servidor da ETAPA 0 é aberto; ver `SECURITY.md`).
- Sentido transmissor → relay e o `KeyframeRequest` repassado ao encoder: ETAPA 3.
- Mudança de resolução em transmissão (novo `VideoInit` + `DISCONTINUITY`): especificado, não implementado.
