# Protocolo de Streaming Vellcast — v1

Status: **v1 implementado** (`crates/protocol`, espelhado em `apps/discord-activity/src/protocol.ts`).
- ETAPA 0: servidor de validação → espectador, sem autenticação (`services/stage0-server`).
- ETAPA 3: transmissor → relay → espectadores, com autenticação na primeira mensagem (`services/relay`, cliente em `crates/transport`). Extensão **aditiva**: o cabeçalho de 28 bytes não mudou.

## 1. Transporte

- Uma mensagem **WebSocket binária** = um pacote do protocolo. Mensagens de texto são ignoradas.
- Em produção, sempre sobre TLS (`wss://`). Na Discord Activity, o WebSocket é aberto na mesma origem (`/ws`) e passa pelo proxy do Discord (ver `DISCORD_INTEGRATION.md`).
- Endpoint da ETAPA 0: `GET /ws?stream=<nome>` (ex.: `1080p60`, `720p30`).
- Endpoint do relay: `GET /ws`, **sem parâmetros**. A identidade vem só da mensagem `Auth` (seção 3.1): tokens nunca vão na URL, que costuma parar em logs de proxy.

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
| `0x20` | `StreamEnd` | T → R → E | Vazio. Do transmissor: fim normal (sala volta a ociosa). Do relay: transmissão encerrada (fim normal, tolerância expirada ou sala fechada). |
| `0x21` | `StreamPaused` | R → E | Vazio. O transmissor caiu sem `StreamEnd`; a sala aguarda a reconexão durante o período de tolerância. |
| `0x30` | `Auth` | C → R | JSON `{ token, role: "publisher" \| "viewer", name? }`. **Primeira mensagem obrigatória**, até 2 KiB, em até 5 s. |
| `0x31` | `AuthOk` | R → C | JSON `{ roomId, role, sessionId, live }`. |
| `0x32` | `Error` | R → C | JSON `{ code, message }`, seguido de close `4000`. Códigos na seção 3.2. |
| `0x40` | `Presence` | R → E | JSON `{ count, participants: [{ id, name, role }] }`: lista **completa** dos participantes ativos da sala. Seção 3.3. |

No relay, `VideoInit`, `VideoFrame`, `AudioInit`, `AudioFrame` e `StreamEnd` são enviados pelo **transmissor** e repassados aos espectadores **byte a byte** (o relay não reescreve cabeçalhos; `sent_at_us` é o relógio do transmissor). `KeyframeRequest` vai do espectador ao relay, que o repassa ao transmissor com limite de 1 a cada 500 ms por sala. `Ping`/`Pong` valem nos dois papéis.

### 3.1 Autenticação

1. O cliente abre `GET /ws` e envia `Auth` como primeira mensagem.
2. O relay valida o token (HMAC-SHA256, expiração), o papel pedido e a sala:
   - `publisher` exige token de papel `publisher` ou `owner`;
   - `viewer` exige token de papel `viewer` ou `owner`.
3. Sucesso → `AuthOk`. Falha → `Error` + close. Qualquer outra mensagem antes de `Auth`, ou `Auth` depois de 5 s, encerra a conexão.

Formato do token: `v1.<payload base64url>.<HMAC-SHA256 base64url>`, payload JSON `{ room, role, exp, nonce, iat, name? }`. A assinatura é verificada em tempo constante **antes** de interpretar o payload. Tokens são emitidos pela API HTTP do relay (`docs/DEPLOYMENT.md` §6).

### 3.2 Códigos de erro

| Código | Significado | Reconectar? |
|---|---|---|
| `unauthorized` | Token ausente, inválido, adulterado ou expirado | Não |
| `forbidden` | Token sem permissão para o papel | Não |
| `room_not_found` / `room_closed` | Sala inexistente ou encerrada | Não |
| `publisher_exists` | Já há outro transmissor (ou esta conexão foi substituída por uma nova com o mesmo token) | Não |
| `protocol_error` / `message_too_large` | Mensagem inválida ou acima do limite | Não |
| `room_full` | Limite de espectadores | Sim, com backoff |
| `auth_timeout` / `idle_timeout` | Sem `Auth` a tempo / sem tráfego | Sim |
| `rate_limited` / `bitrate_exceeded` | Taxa de conexões ou banda de entrada acima do limite | Sim, com backoff (e menor bitrate) |
| `internal` | Erro do servidor | Sim, com backoff |

`ErrorCode::is_fatal()` (crate `vellcast-protocol`) implementa essa tabela.

Tipos desconhecidos são rejeitados por quem lê (`DecodeError::UnknownType`); por isso, um tipo novo exige nova versão **ou** negociação explícita — ou, como em `Presence` (seção 3.3), ser enviado só a clientes que comprovadamente ignoram tipos desconhecidos.

### 3.3 Participantes (`Presence`)

Lista de quem está na sala, para o player mostrar nomes e contagem (issue #3).

- **Nome do convidado:** `POST /api/rooms/{id}/join` aceita `{ invite, name? }`. O relay remove caracteres de controle, apara espaços e exige de 1 a 32 caracteres (`sanitize_participant_name`); um nome inválido recebe `400 protocol_error` **antes** de o convite ser consumido. O nome vai assinado no token de espectador (campo `name`), então o espectador não consegue trocá-lo depois. `name` ausente continua aceito (clientes antigos) e aparece como `null` na lista.
- **Nome do transmissor:** campo opcional `name` no `Auth` (`AuthProfile`), lido à parte de `AuthRequest`; relays antigos o ignoram. Para espectadores o campo é ignorado (vale o nome do token). O app do Windows envia o nome de exibição das configurações.
- **Sessões, não conexões:** cada entrada é um token (`nonce`) em um papel. Duas conexões com o mesmo token — uma reconexão antes de a conexão antiga cair, ou a página recarregada na mesma aba — contam como **uma** entrada; ela sai da lista quando a última conexão termina (fechamento, queda, inatividade, revogação ou sala encerrada). O dono assistindo e transmitindo com o mesmo token aparece duas vezes (um por papel).
- **`id`** é aleatório, criado na entrada da sessão e estável enquanto ela durar. Não deriva do token nem do convite; nenhum dos dois aparece no payload.
- **Envio:** o espectador recebe a lista atual logo após o `AuthOk` (já incluindo a si mesmo) e uma nova lista a cada entrada, saída ou troca de nome. Mudanças em rajada são agrupadas (só a versão mais recente é enviada). A lista é ordenada: transmissor primeiro, depois espectadores por ordem de chegada, limitada a 100 itens (`count` é sempre o total).
- **`name` é texto não confiável:** pode conter `<`, `&` etc. Clientes devem exibi-lo como texto puro (no player, React escapa; nunca `dangerouslySetInnerHTML`).
- **Compatibilidade:** só espectadores recebem `Presence`. O player web (`connection.ts`/`player.ts`) ignora tipos que não conhece, então players antigos seguem funcionando; o transmissor (`crates/transport`) também descarta mensagens que não decodifica, mas não recebe `Presence`.

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

### 7.1 Servidor da ETAPA 0

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

### 7.2 Relay (`services/relay`, padrões configuráveis em `config.rs`)

| Limite | Padrão |
|---|---|
| Mensagem do transmissor | 4 MiB (limite do WebSocket; acima disso a conexão é fechada) |
| Mensagem do espectador | 1 KiB (`message_too_large`) |
| `Auth` | 2 KiB, em até 5 s |
| `VideoInit`/`AudioInit` | 1 KiB, JSON validado (codec ≤ 64 caracteres, dimensões > 0) antes de repassar |
| Fila por espectador | 256 mensagens; ao estourar, perde as mais antigas e volta no próximo keyframe |
| Idade máxima de quadro na fila do relay | 500 ms |
| Envio travado para um espectador | 10 s → desconectado (`slowViewersDisconnected`) |
| Cache do GOP atual | 8 MiB (acima disso, novos espectadores aguardam o próximo keyframe) |
| Banda de entrada por transmissor | 40 Mbps, rajada de 2 s (`bitrate_exceeded`) |
| Espectadores por sala | 50 (o dono pode reduzir na criação) |
| Salas por servidor | 1000 |
| Inatividade | 30 s sem mensagens → `idle_timeout` |
| Tolerância a queda do transmissor | 30 s em `StreamPaused` |

### 7.3 Novo espectador no relay

O relay guarda `VideoInit`, `AudioInit` e o **GOP atual** (do último keyframe em diante, só vídeo). Um espectador novo recebe `AuthOk` → configuração → GOP e então o fluxo ao vivo, sem esperar o próximo keyframe. Inscrição e snapshot acontecem sob o mesmo lock da publicação, então não há lacunas nem quadros duplicados. Sem GOP em cache, o relay pede um keyframe ao transmissor.

### 7.4 Transmissor (`crates/transport`)

- Fila de envio limitada (240 mensagens / 8 MiB). Sob pressão, descarta deltas até o próximo keyframe e emite `KeyframeRequested` para o encoder; um keyframe novo descarta o vídeo obsoleto ainda na fila; áudio descarta os pacotes mais antigos.
- Reconexão com backoff 0,5 s → 8 s e jitter de ±20%. Após reconectar, reenvia `VideoInit`/`AudioInit` com a flag `DISCONTINUITY` e pede um keyframe ao encoder.
- Uma nova conexão com o **mesmo token** substitui a anterior (útil quando a conexão antiga ainda não caiu do lado do relay); um token diferente recebe `publisher_exists`.

## 8. Versionamento

- O campo `version` muda quando o layout do cabeçalho ou a semântica de um tipo existente muda.
- Campos e flags reservados permitem extensões compatíveis dentro da v1.
- A ETAPA 3 estendeu a v1 de forma aditiva (tipos `0x21`, `0x30`–`0x32`); a lista de participantes acrescentou `0x40` (`Presence`), enviado só a espectadores. Clientes antigos da Activity ignoram tipos que não conhecem no player, mas `Header::read` em Rust rejeita tipos desconhecidos: transmissor e relay precisam ser atualizados juntos.
- A negociação explícita de versão (versões suportadas no `Auth`) fica para quando houver clientes distribuídos de forma independente (ETAPA 5/8).

## 9. Pendências conhecidas

- O player da Activity ainda usa o servidor da ETAPA 0 (sem `Auth`). A troca para o relay entra com o pareamento (ETAPA 5).
- Mudança de resolução durante a transmissão: o relay já repassa um novo `VideoInit` e reinicia o cache; falta o encoder real (ETAPA 2) e o teste no player.
- `KeyframeRequest` chega ao transmissor como evento; gerar o IDR depende do encoder (ETAPA 2).
