# Desempenho: metas, metodologia e resultados

> **Regra:** nenhum valor desta página é considerado atingido sem uma medição reproduzível registrada com hardware, sistema, rede e metodologia. Os valores abaixo são **metas de engenharia**, não garantias. O Vellcast não promete impacto zero no desempenho dos jogos.

## 1. Metas

| Cenário | Métrica | Meta |
|---|---|---|
| App ocioso | CPU | < 1% |
| App ocioso | RAM (working set) | < 80 MB |
| App ocioso | GPU | Atividade mínima, sem redraw contínuo |
| App minimizado | Renderização | Nenhum frame da UI desenhado |
| Transmissão | Resolução / FPS | 1920×1080 a 60 |
| Transmissão | Codec / encoder | H.264 por hardware |
| Transmissão | Bitrate de referência | 8 Mbps (configurável) |
| Impacto no jogo | FPS médio | Perda < 5% em hardware de referência |
| Impacto no jogo | 1% lows, frametime, stutter | Medir e reportar (meta a definir após a linha de base) |
| Latência | Ponta a ponta (captura → tela do espectador) | < 500 ms em condições favoráveis |
| Estabilidade | Sessão contínua | ≥ 60 min sem crescimento de memória nem degradação |

## 2. Metodologia

### 2.1 Regras gerais
- Sempre em build **Release**.
- Registrar: CPU, GPU, driver, RAM, versão do Windows/SO, resolução e taxa do monitor, versão do Vellcast (commit), rede (tipo, RTT, perda) e cliente Discord (Desktop/Web/versão).
- Pelo menos 3 execuções por cenário. Reportar mediana e dispersão.
- Coletar os dados brutos em `tests/benchmarks/` (CSV/JSON) junto com o script usado.

### 2.2 Impacto no jogo (ETAPA 6)
- Ferramenta: PresentMon (FPS, frametime, 1% lows).
- Protocolo A/B: o mesmo trecho de jogo/benchmark interno (a) sem Vellcast, (b) com Vellcast aberto e ocioso e (c) transmitindo 1080p60 a 8 Mbps.
- Coletar também o uso de CPU, GPU e do encoder (contadores da GPU no Gerenciador de Tarefas/PDH).

### 2.2.1 Adaptação de qualidade (S-001/S-002)

- A lógica é validada por testes unitários com sinais sintéticos (`cargo test -p vellcast-core adapt`).
- No Windows real, registrar para cada GPU: se o encoder aceita mudar o bitrate em tempo real (log "o encoder não aceita mudar o bitrate em tempo real" indica que não), o tempo até a primeira redução após limitar a banda (ex.: `clumsy` ou limite no roteador) e o tempo de recuperação após liberar a banda.
- Modo competitivo: comparar FPS médio e 1% lows do jogo com perfil Equilibrado vs Competitivo, no mesmo trecho do jogo.

### 2.3 Latência
- **ETAPA 0 (parcial):** `t_frame_decodificado − sent_at_us`, convertidos para o relógio do servidor via offset Ping/Pong no estilo NTP, usando a amostra de menor RTT. Captura, encoder e renderização no transmissor ficam de fora (ADR-010).
- **ETAPA 5+ (ponta a ponta):** Relógio de alta resolução renderizado na fonte e fotografado/gravado junto com a tela do espectador (glass-to-glass), ou timestamp embutido no frame e comparado na exibição com relógios sincronizados.

### 2.4 Estabilidade
- Sessões de 60 min com amostragem de RAM/CPU a cada 10 s, desconexões forçadas (transmissor e espectador) e um espectador artificialmente lento.
- Critério: memória estável (sem tendência de crescimento) e recuperação automática.

### 2.5 Relay
- Teste de carga com N espectadores sintéticos (cliente WS headless): CPU, RAM, banda de saída e atraso entre receber e reenviar.
- Verificar que um espectador lento não altera a latência dos demais.

## 3. Resultados

As tabelas são preenchidas somente com dados reais. Resultados de ambiente local **não** substituem a validação no Discord.

**Ambiente da medição local de 2026-10-08:** Apple M1 (8 GB), macOS 27.0.1, servidor `vellcast-stage0-server` em build Release, cliente na mesma máquina (loopback), Chromium headless 151 (Playwright, via gstack browse). Mídia: `scripts/generate-test-media.sh` (x264 `veryfast`/`zerolatency`, High, sem B-frames, GOP 1 s; Opus 96 kbps, 20 ms). A latência em loopback mede só servidor → decodificação, sem rede real.

### 3.1 ETAPA 0 — Reprodução na Discord Activity

| Cenário | Cliente | FPS decodificado | Frames descartados | Latência servidor→decodificado (p50/p95) | Áudio | Status |
|---|---|---|---|---|---|---|
| 720p30 H.264 | Discord Desktop | — | — | — | — | Pendente |
| 1080p60 H.264 | Discord Desktop | — | — | — | — | Pendente |
| 720p30 H.264 | Discord Web | — | — | — | — | Pendente |
| 1080p60 H.264 | Discord Web | — | — | — | — | Pendente |
| 1080p60 H.264 | Discord móvel | — | — | — | — | Pendente |
| 720p30 H.264 (4,0 Mbps) | Chromium headless local (fora do Discord) | 30/30 | 0 | média 3 / p95 4 ms | Opus suportado | Medido (local) |
| 1080p60 H.264 (8,1 Mbps) | Chromium headless local (fora do Discord) | 60/60 | 0 | média 3 / p95 3 ms | Opus suportado; A/V skew estável ~73–77 ms (1 amostra isolada de −109 ms em 10) | Medido (local) |

| Verificação | Resultado | Status |
|---|---|---|
| `VideoDecoder.isConfigSupported` (avc1) no Discord | Local: `avc1.64002A` e `avc1.64001F` suportados no Chromium headless | Pendente no Discord |
| `AudioDecoder.isConfigSupported` (opus) no Discord | Local: suportado | Pendente no Discord |
| Reconexão após queda do servidor | Local: servidor encerrado por ~4 s e reiniciado; o player reconectou com backoff e voltou a 30/30 fps (reconexões = 1) | Medido (local); Discord pendente |
| Gamepad API dentro da Activity | Local: `navigator.getGamepads` presente e Permissions Policy permite; nenhum controle físico testado | Pendente |

**Bloqueio:** a validação dentro do Discord exige uma aplicação Discord (client ID) com Activities habilitadas e um túnel HTTPS público. Enquanto isso não existir, o status fica **Blocked / Pending Validation**.

### 3.2 ETAPA 1 — App desktop

| Cenário | CPU | RAM | Status |
|---|---|---|---|
| Ocioso, janela visível (renderer de software) — macOS | 0,05–0,27% de 1 núcleo | 32–33 MB (`phys_footprint`) | Medido no macOS; Windows pendente |
| Ocioso, oculto na bandeja — macOS | 0,03–0,10% de 1 núcleo | 20 MB (`phys_footprint`) | Medido no macOS; Windows pendente |
| Ocioso, janela visível (femtovg/OpenGL) — macOS | 0,05% de 1 núcleo | 41 MB (`phys_footprint`) | Medido no macOS (referência para o ADR-014) |
| Ocioso, janela visível — Windows 11 | — | — | Pendente (máquina Windows) |
| Ocioso, minimizado/bandeja — Windows 11 | — | — | Pendente (máquina Windows) |

Metodologia (macOS): Apple M1 8 GB, macOS 27.0.1, build Release, 20–30 s ociosos, amostragem por `ps` (CPU) e `footprint` (memória). O RSS do `ps` (~80–100 MB) inclui páginas compartilhadas de frameworks do macOS e não representa o custo real do app. A máquina estava sob carga (load average entre 8 e 18) durante as medições. Ver `apps/desktop/README.md`.

### 3.3 ETAPA 2/6 — Captura, encoder e impacto no jogo

**Captura e áudio em Windows real (runner do GitHub, workflows `windows-e2e` e `diag-audio`, 2026-10-08):**
- WGC de monitor (1024×768) e de janela (Bloco de Notas) → H.264 pelo `H264 Encoder MFT` com conversão na CPU: arquivos começam com SPS (High) e foram gerados sem erros.
- Áudio WASAPI: o primeiro teste derrubava o processo (`0xC0000374`, corrupção de heap). O diagnóstico sob `cdb` com page heap apontou liberação dupla do blob de `AUDIOCLIENT_ACTIVATION_PARAMS` (a ativação do Windows libera o blob). Corrigido com `CoTaskMemAlloc`; depois da correção, loopback do sistema (excluindo o Discord): 196 pacotes Opus em 4 s e loopback por processo: 149 pacotes em 3 s, intervalos de 20 ms exatos, sem alertas do verificador de heap.
- Fonte parada: o WGC não entrega quadros sem mudança na tela; o pipeline passou a repetir o último quadro no ritmo do FPS-alvo.
- **Bitrate adaptativo no Windows real (S-001, `tests/e2e/abr-check.mjs`):** monitor com console rolando texto, subida limitada a 300 kbps por proxy TCP. Alvo: 3000 → 2100 → 1470 → 504 → 500 kbps (piso); bitrate codificado acompanhou em tempo real: 2075 → 1417 → 866 → 461 → ~350–560 kbps (o encoder de software do Media Foundation aceitou a mudança ao vivo); nenhum quadro descartado na rede; o usuário viu os avisos "Rede lenta até o servidor: qualidade reduzida…". Encoders de hardware ainda precisam ser verificados com GPU real.
- **Sincronia A/V no transporte (E4-T05):** diferença entre o atraso mediano do áudio e o do vídeo com o mesmo PTS, medida nos espectadores: 9,2 ms e 12,4 ms em duas execuções (limite do teste: 250 ms).
- **Sessão de 60 minutos no Windows real (M-025, `windows-e2e` com `seconds=3600`, run 37864516247):** monitor 1024×768 + áudio do sistema → relay → 3 espectadores, encoder de software (runner sem GPU, 2 vCPUs). Vídeo: 15,9 fps constantes por espectador, 956 keyframes, nenhum erro de protocolo. Áudio: 180.000 pacotes por espectador (50/s durante a hora inteira). Memória do transmissor (working set, amostra a cada 10 s): 67 MB no início, ~33 MB ao final — sem crescimento. Ponto a observar: maior intervalo entre quadros de 10,3 s, ocorrido uma vez na hora; não aparece em sessões curtas e deve ser reavaliado com GPU real.

**Execução em Windows (runner do GitHub Actions, sem GPU, 2026-10-08):** a enumeração listou 1 janela e 1 monitor (1024×768) e o encoder `H264 Encoder MFT` (software). O teste `encodes_synthetic_frames_with_sps_idr_and_forced_keyframe` passou com conversão BGRA→NV12 na CPU (ADR-031): saída começa com SPS+IDR e o keyframe sob demanda funciona. Não mede desempenho; números com GPU e jogos continuam pendentes.

| Jogo/benchmark | Hardware | FPS médio sem / com | 1% lows sem / com | Encoder | Status |
|---|---|---|---|---|---|
| — | — | — | — | — | Pendente |

### 3.4 Relay

| Espectadores | Bitrate | CPU | RAM | Banda de saída | Status |
|---|---|---|---|---|---|
| 20 (loopback, `tests/benchmarks/stage0-viewers.mjs`, 20 s) | 7,9 Mbps por espectador | média 4,4% de 1 núcleo | RSS máx. 9,6 MB | ~158 Mbps | Medido (local, servidor da ETAPA 0) |
| 20 autenticados por convite (loopback, `tests/benchmarks/relay-viewers.mjs`, 30 s, transmissor `publish_fixture` 1080p60) | 8,21 Mbps por espectador | 4,8% de 1 núcleo | RSS máx. 9,7 MB | ~164 Mbps | Medido (local, `vellcast-relay`) |

Produção (`https://vellcast.nextestudios.com`, VPS na Europa, cliente no Brasil, 2026-10-08): a rota limita uma conexão a 1,2–2,2 Mbps (RTT ~290 ms), então 720p30 a 4 Mbps chegou a 15 fps com 1 espectador e a 2–8 fps com 5. Gargalo de rede, não do relay; ver `DEPLOYMENT.md` §6.10.

**Estabilidade (M-025), 2026-10-08:** `vellcast-relay` em Release, 1 transmissor 1080p60 (`publish_fixture`) e 20 espectadores autenticados por **60 minutos** (`tests/benchmarks/relay-viewers.mjs … 20 3600`), Apple M1, loopback. Resultado: todos os espectadores a 60,0 fps e 8,17 Mbps; 75 GB entregues; `laggedMessages = 0`, `staleFramesDropped = 0`, `slowViewersDisconnected = 0`, nenhum erro; latência transmissor → espectador p50 41 ms / p95 77 ms / p99 80 ms; maior intervalo entre quadros 308 ms. Memória do relay (RSS, amostra a cada 30 s): 3–11 MB, estável do início ao fim (sem crescimento); CPU média 8% de 1 núcleo.

Detalhes do teste do `vellcast-relay`: 60,3 fps por espectador, maior intervalo entre quadros 113,5 ms, latência transmissor → espectador p50 0,85 ms / p95 2,23 ms / p99 9,10 ms; nenhum erro, mensagem atrasada (`lagged`), quadro velho descartado ou desconexão. Loopback sem TLS: não mede rede real nem a CPU do transmissor.

Detalhes do teste de 20 espectadores: 58,0 fps por espectador (os primeiros ~0,7 s aguardam o primeiro keyframe), maior intervalo entre quadros 71 ms, latência servidor → recebimento p50 1,2 ms / p95 3,9 ms / p99 17,8 ms, nenhum fechamento anormal, `laggedMessages = 0`, `staleFramesDropped = 0`. É um teste de loopback: não mede rede real, perda de pacotes nem proxy do Discord.

Observação: quando a aba do navegador fica em segundo plano, o Chromium reduz o ritmo da página. Nesse estado foram observados pedidos de keyframe por fila cheia no decoder e A/V skew de ~265 ms. É o comportamento esperado do mecanismo de descarte, não uma medição de primeiro plano.

### 3.5 Benchmarks reproduzíveis do relay (E6-T01/E6-T02)

Ferramentas em `crates/perf` (ver `crates/perf/README.md`). Todas em `--release`, Apple M1 8 GB, macOS 27.0.1, loopback, com outros builds rodando na máquina (load average 3–14 durante as medições; a CPU varia por isso).

**`fanout_bench`**: relay em processo, em runtime próprio com 2 threads; 1 transmissor sintético 1080p60 a 8 Mbps (keyframe a cada 2 s) e 20 espectadores WebSocket reais; 2 s de aquecimento e 20 s de medição. Um alocador contador soma só as alocações feitas nas threads do relay.

```bash
cargo run --release -p vellcast-perf --bin fanout_bench -- 20 20 60 8000 off   # uma escrita por mensagem (comportamento anterior)
cargo run --release -p vellcast-perf --bin fanout_bench -- 20 20 60 8000 on    # escrita em lote (padrão)
```

| Versão | Alocações do relay por mensagem entregue | Bytes alocados por mensagem | Latência p50 / p95 / p99 | CPU do processo (relay + clientes) | FPS por espectador |
|---|---|---|---|---|---|
| Primeira versão do lote (`Vec` novo por mensagem) | 1,02–1,03 | 760–873 B | 0,36–0,52 / 1,1–3,8 / 2,4–8,7 ms | 7,6–12,3% | 60,0 |
| Antes: escrita por mensagem (`batch=off`, 2 execuções) | **0,03** | 630–632 B | 0,46–0,51 / 1,3–1,7 / **2,3–3,0 ms** | 12,6–13,6% | 60,0 |
| Depois: escrita em lote (`batch=on`, 2 execuções) | **0,03** | 712–746 B | 0,62–0,63 / 1,16–1,17 / **1,69–1,73 ms** | 14,1–14,8% | 60,0 |

Leitura:
- O caminho quente do relay já não copiava mídia (o mesmo `Bytes` vai a todos os espectadores). A medição confirma: **0,03 alocação por mensagem entregue**, que é o buffer de cada mensagem recebida do transmissor (uma por mensagem de entrada, dividida entre 20 espectadores). Não há alocação por espectador.
- A medição pegou uma regressão na primeira versão do lote (um `Vec` alocado a cada mensagem, 1,02 alocação/mensagem); corrigida reaproveitando o vetor.
- O lote (`feed` de todas as mensagens pendentes + um `flush`) reduziu o p99 de 2,3–3,0 ms para ~1,7 ms. A CPU ficou dentro do ruído da máquina carregada; o benefício maior é esperado quando um espectador acumula fila (rede lenta) e no envio do snapshot inicial (configuração + GOP em cache, até centenas de mensagens), que também passou a ser uma única escrita. O transmissor (`crates/transport`) passou a drenar sua fila da mesma forma.
- 44 mil mensagens entregues em 20 s (~163 Mbps), sem `lagged` nem quadros velhos descartados.

### 3.6 Avaliação QUIC × WebSocket/TCP (E6-T05, S-005)

**`quic_eval`**: transmissor sintético 1080p60 a 8 Mbps → receptor, por um proxy de perturbação no mesmo processo, 10 s por cenário. QUIC com Quinn 0.11.12, um stream unidirecional por quadro com prioridade FIFO, com Cubic (padrão) e com BBR. O modelo de TCP preserva a ordem e transforma perdas em atraso de retransmissão, mas **não reduz a janela de congestionamento**: os números de TCP com perda são otimistas.

```bash
cargo run --release -p vellcast-perf --bin quic_eval -- 10
```

| Cenário (atraso de ida, jitter, perda) | Transporte | Quadros entregues | Latência p50 / p95 / p99 (ms) | Recuperação após apagão de 300 ms | CPU do processo |
|---|---|---|---|---|---|
| limpo (20 ms, 2 ms, 0%) | WebSocket/TCP | 600/600 | 23,5 / 24,9 / 25,7 | — | 1,8% |
| | QUIC Cubic | 600/600 | 24,3 / 28,2 / 189,9 | — | 10,7% |
| | QUIC BBR | 600/600 | 24,1 / 25,7 / 34,4 | — | 10,1% |
| perda 1% (20 ms, 2 ms) | WebSocket/TCP (otimista) | 600/600 | 24,0 / 73,3 / 74,6 | — | 1,6% |
| | QUIC Cubic | 315/600 | 1762 / 5471 / 5902 | — | 7,0% |
| | QUIC BBR | 600/600 | 24,1 / 69,9 / 82,6 | — | 10,3% |
| perda 3% (40 ms, 5 ms) | WebSocket/TCP (otimista) | 600/600 | 116 / 136 / 137 | — | 1,3% |
| | QUIC Cubic | 81/600 | 4174 / 9547 / 10031 | — | 3,5% |
| | QUIC BBR | 600/600 | 47,5 / 147 / 233 | — | 12,3% |
| apagão de 300 ms em t = 4 s (20 ms, 2 ms) | WebSocket/TCP | 600/600 | 23,5 / 32,7 / 417 | 203 ms | 1,7% |
| | QUIC Cubic | 600/600 | 24,6 / 435 / 720 | 781 ms | 11,0% |
| | QUIC BBR | 600/600 | 24,3 / 26,6 / 293 | **27 ms** | 11,0% |

RSS máximo do processo: 4–29 MB (cresce ao longo dos cenários; o processo é o mesmo). Leitura e decisão em `DECISIONS.md` (ADR-037).

