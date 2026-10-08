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

Detalhes do teste do `vellcast-relay`: 60,3 fps por espectador, maior intervalo entre quadros 113,5 ms, latência transmissor → espectador p50 0,85 ms / p95 2,23 ms / p99 9,10 ms; nenhum erro, mensagem atrasada (`lagged`), quadro velho descartado ou desconexão. Loopback sem TLS: não mede rede real nem a CPU do transmissor.

Detalhes do teste de 20 espectadores: 58,0 fps por espectador (os primeiros ~0,7 s aguardam o primeiro keyframe), maior intervalo entre quadros 71 ms, latência servidor → recebimento p50 1,2 ms / p95 3,9 ms / p99 17,8 ms, nenhum fechamento anormal, `laggedMessages = 0`, `staleFramesDropped = 0`. É um teste de loopback: não mede rede real, perda de pacotes nem proxy do Discord.

Observação: quando a aba do navegador fica em segundo plano, o Chromium reduz o ritmo da página. Nesse estado foram observados pedidos de keyframe por fila cheia no decoder e A/V skew de ~265 ms. É o comportamento esperado do mecanismo de descarte, não uma medição de primeiro plano.
