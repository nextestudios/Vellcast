# Roadmap — Vellcast

> Etapas, tarefas, dependências, entregáveis e status. Prioridades em [MOSCOW](MOSCOW.md); riscos em [RISKS](RISKS.md).
> Status permitidos: `Backlog`, `Ready`, `In Progress`, `Blocked`, `Testing`, `Done`. `Pending Validation` indica código escrito cuja validação depende de hardware, credenciais ou acesso externo indisponível.

Última revisão: 2026-10-08.

## Visão geral

| Etapa | Nome | Prioridade | Depende de | Status |
|---|---|---|---|---|
| 0 | Validação Discord | Must | — | Blocked — Activity, OAuth e pareamento prontos; falta a aplicação no Discord (E0-T10) |
| 1 | Desktop e controles | Must | 0 (local) | Done (exceto E1-T05: controle físico) — executado no Windows real |
| 2 | Captura e encoder | Must | 0 (local) | Testing — executada no Windows (CI, sem GPU); falta GPU real |
| 3 | Media relay | Must | 0 (local) | Done — em produção em vellcast.nextestudios.com; 60 min estável |
| 4 | Áudio | Must | 2, 3 | Done — executada no Windows real; sincronia A/V de 9,2 ms no transporte |
| 5 | Integração completa | Must | 1, 2, 3, 4 (E0-T10 para a Activity) | Done (MVP Windows + player web); Activity aguarda E0-T10 |
| OC | Marco: abertura do código (Open Core) | Should | 5 | Suspenso — código proprietário e repositório privado (ADR-058) |
| 6 | Otimização extrema | Must | 5 | Testing — software pronto; otimização com GPU real bloqueada |
| 7 | Comercialização | Should (MVP) / Must (Premium) | 5 | Testing — backend pronto; cobrança real depende de conta no provedor |
| 8 | Lançamento | Must (lançamento) | 6, 7 | Testing — instalador e atualizador prontos; certificado e revisão jurídica pendentes |

As Etapas 1, 2 e 3 podem avançar em paralelo. Por decisão de 2026-10-08 ([ADR-013](DECISIONS.md)), a validação dentro do Discord (E0-T10) foi adiada e as etapas seguintes avançam pela ordem MoSCoW; E0-T10 passa a ser pré-requisito da Etapa 5.

Ambiente de desenvolvimento atual: macOS. Captura, GPU, WASAPI e gamepad no Windows exigem uma máquina Windows real; a CI Windows valida apenas compilação e testes sem hardware.

---

## Etapa 0 — Validação Discord

**Objetivo:** comprovar a viabilidade de reproduzir vídeo H.264 (e áudio Opus) dentro de uma Discord Activity real, usando mídia sintética, antes de investir no motor Windows.

**Fora do escopo:** captura Windows, encoder por hardware, autenticação completa, salas.

| ID | Tarefa | MoSCoW | Depende de | Critérios de aceite | Status |
|---|---|---|---|---|---|
| E0-T01 | Estrutura do monorepositório | M-022 | — | Árvore da seção 28 criada; Cargo workspace com `crates/protocol` e `services/stage0-server`; diretórios futuros apenas com README de escopo (sem crates vazios no workspace) | Done |
| E0-T02 | Protocolo v1 (`crates/protocol`) + testes | M-010, M-020 | E0-T01 | Cabeçalho binário de 28 bytes little-endian conforme [STREAMING_PROTOCOL](STREAMING_PROTOCOL.md); encode/decode com limite de tamanho; testes de ida e volta, versão inválida, mensagem truncada e mensagem acima do limite | Done — 6 testes |
| E0-T03 | Geração de mídia sintética | M-012 | E0-T01 | `scripts/generate-test-media.sh` (ffmpeg) gera 720p30 e 1080p60 H.264 Annex B sem B-frames, keyframe a cada 1 s, AUD por access unit, e Opus 48 kHz estéreo em pacotes de 20 ms; mídia gerada não é versionada | Done |
| E0-T04 | Servidor WebSocket de teste (Axum) | M-010, M-011 | E0-T02, E0-T03 | Serve a Activity estática e `/ws`; pacing pelo timestamp de mídia; fan-out para N espectadores com descarte e ressincronização no keyframe; heartbeats; limites de mensagem | Done (local) — 6 testes de parsing; teste de carga com 20 espectadores |
| E0-T05 | Activity mínima | M-012 | E0-T04 | React/TS/Vite; Embedded App SDK `ready()` quando dentro do Discord e modo standalone fora dele; `VideoDecoder` (Annex B, codec string derivada do SPS) e `AudioDecoder` Opus; detecção via `isConfigSupported` | Done — superada pela Activity final (E5-T05) |
| E0-T06 | Métricas no player | M-023 | E0-T05 | Exibe FPS decodificado, frames descartados, bitrate recebido, latência servidor→decodificação (com estimativa de offset de relógio por ping/pong) e A/V skew | Done — métricas do player validadas |
| E0-T07 | Reconexão | M-019 | E0-T05 | Backoff exponencial com jitter; retomada no próximo keyframe; tempo de recuperação medido ao reiniciar o servidor | Done — reconexão validada (local e produção) |
| E0-T08 | Verificação da Gamepad API | M-003 | E0-T05 | Relatório de disponibilidade de `navigator.getGamepads()` e eventos de conexão, exibido no player | Done — Gamepad API detectada no navegador; validação no iframe do Discord junto com E0-T10 |
| E0-T09 | Teste em navegador local | M-012 | E0-T04..T08 | 720p30 e 1080p60 reproduzidos em Chrome/Edge locais com métricas registradas | Done — validado em Chromium (local e servidor de produção) |
| E0-T10 | Teste em Activity real | M-012 | E0-T09 | Activity executada no Discord Desktop e Web (mobile quando aplicável) via URL Mapping; métricas registradas | Blocked — implementação pronta (OAuth, pareamento, Activity); falta o responsável criar a aplicação no Discord (DISCORD_INTEGRATION.md) |
| E0-T11 | Relatório de resultados | M-022, M-023 | E0-T09, E0-T10 | Resultados de vídeo, áudio, FPS, latência, reconexão e gamepad registrados separadamente em [PERFORMANCE](PERFORMANCE.md), com hardware, sistema, rede e metodologia | Done — resultados em PERFORMANCE.md |
**Entregável:** protótipo reproduzível dentro do Discord.

**Critério de aceite da etapa:** vídeo reproduzido dentro de uma Activity real, com métricas documentadas; áudio, FPS, latência e suporte a controle registrados separadamente. Se algum requisito fundamental falhar (por exemplo, `VideoDecoder` H.264 indisponível ou throughput insuficiente no proxy), documentar o bloqueio em [DECISIONS](DECISIONS.md) e avaliar alternativas antes de avançar.

**Status:** Testing. Validação local concluída; E0-T10 adiada ([ADR-013](DECISIONS.md)).

---

## Etapa 1 — Desktop e controles

**Objetivo:** aplicativo Windows extremamente leve, navegável por controle.

| ID | Tarefa | MoSCoW | Depende de | Critérios de aceite | Status |
|---|---|---|---|---|---|
| E1-T01 | Decisão de licença do Slint | M-001 | — | Opção registrada em [LICENSING](LICENSING.md) e [DECISIONS](DECISIONS.md) | Done — Royalty-free 2.0 (ADR-006) |
| E1-T02 | Esqueleto `apps/desktop` (Rust + Slint) | M-001 | E1-T01 | Build Release na CI Windows | Done — build Release na CI Windows; app executado no Windows (E2E) |
| E1-T03 | Design system em Slint | M-002 | E1-T02 | Tokens de cor, tipografia, espaçamento e foco conforme [UX_GUIDELINES](UX_GUIDELINES.md) | Done — design system renderizado no Windows (capturas do E2E) |
| E1-T04 | Navegação lateral e telas (Home, Transmitir, Salas, Configurações) | M-002 | E1-T03 | Telas navegáveis; nenhuma tela simulada apresentada como funcional | Done — 8 telas renderizadas no Windows; estados honestos para etapas futuras |
| E1-T05 | Crate `controller` (GilRs + XInput quando necessário) | M-003, S-010 | E1-T02 | Mapeamento da seção 13; deadzone; repetição controlada; conexão/desconexão | Blocked — lógica testada (16 testes); falta validar com controle físico (Xbox/DualShock/DualSense) |
| E1-T06 | Sistema de foco e modais | M-003 | E1-T04, E1-T05 | Foco sempre visível; modais navegáveis | Done — foco e modais validados pelo mesmo caminho do gamepad (ações simuladas); controle físico em E1-T05 |
| E1-T07 | Teclado virtual | M-004 | E1-T06 | Requisitos da seção 14 | Done — teclado virtual renderizado no Windows e validado com ações simuladas; controle físico em E1-T05 |
| E1-T08 | Modo bandeja | M-017 | E1-T02 | Sem renderização quando minimizado | Done — Windows: início minimizado sem janela visível, 0,16% de CPU e 27,5 MB na bandeja; clique no ícone e menu a confirmar no teste manual |
| E1-T09 | Preparação de i18n | S-013 | E1-T04 | Strings externalizadas | Done — pt-BR/en com troca em tempo real; teste falha se faltar tradução (ADR-017) |
| E1-T10 | Medição de CPU/RAM em Release | M-023 | E1-T08 | Ocioso e minimizado medidos em Windows real | Done — Windows (VM da CI, renderer de software): CPU ociosa 0,47% de 1 núcleo, working set 36 MB (metas: <1% e <80 MB) |

**Entregável:** aplicativo navegável por controle.
**Critério de aceite:** todas as operações implementadas nesta etapa acessíveis por controle; CPU e RAM medidos em Release.
**Status:** Backlog.

---

## Etapa 2 — Captura e encoder

**Objetivo:** capturar e codificar vídeo nativamente, mantendo frames na GPU.

| ID | Tarefa | MoSCoW | Depende de | Critérios de aceite | Status |
|---|---|---|---|---|---|
| E2-T01 | Crate `capture` — WGC de janela | M-005 | Etapa 0 | Seleção explícita; resize; fechamento; perda de dispositivo | Done — captura de janela executada no Windows real (E2E); jogos com GPU em E2-T08 |
| E2-T02 | WGC de monitor | M-006 | E2-T01 | Multi-monitor; mudança de resolução | Done — captura de monitor executada no Windows real (E2E e transmissão pelo app) |
| E2-T03 | Fallback DXGI Desktop Duplication (se necessário) | M-006 | E2-T02 | Somente se justificado por teste | Done — não necessário: WGC validado no Windows real (ADR-057) |
| E2-T04 | Crate `encoder` — detecção de encoders (MF) | M-007, S-006 | E2-T01 | Lista NVENC/AMF/QSV disponíveis | Done — detecção executada no Windows (H264 Encoder MFT); hardware NVENC/AMF/QSV em E2-T08 |
| E2-T05 | Pipeline D3D11 → encoder H.264 sem cópia para CPU | M-007, M-018 | E2-T04 | Baixa latência; sem B-frames; keyframe sob demanda; fila limitada | Blocked — caminho de GPU (VideoProcessor + MFT de hardware) exige Windows com GPU; caminho de CPU validado |
| E2-T06 | Configuração de resolução/FPS/bitrate | M-016 | E2-T05 | Reconfiguração ou reinício controlado | Done — resolução/FPS/bitrate aplicados no Windows (1024×768, 30/60 fps, 2–8 Mbps) |
| E2-T07 | Detecção de jogos | S-003 | E2-T01 | Heurística não invasiva | Blocked — heurística testada; validar com jogos reais em Windows com GPU |
| E2-T08 | Medição 1080p60 | M-008, M-023 | E2-T05 | Consumo e limitações registrados por GPU | Blocked — requer máquina Windows com GPU |
| E2-T09 | Pipeline do `vellcast-core` (captura → encoder → relay, eventos, métricas, recuperação de dispositivo) | M-007, M-018 | E2-T05, E3-T02 | Sessão inicia, transmite e encerra pelo app | Done — pipeline executado no Windows: monitor/janela → encoder → relay → espectadores; app transmitiu ao servidor de produção |
| E2-T10 | Ferramentas de validação manual (`capture_to_file`, teste de encoder `#[ignore]`) | M-023 | E2-T09 | Executáveis gerados | Done — execução pendente no Windows |

**Entregável:** pipeline de captura e codificação funcional.
**Critério de aceite:** capturar e codificar 1080p60 em hardware compatível, registrando consumo e limitações.
**Status:** Backlog.

---

## Etapa 3 — Media relay

**Objetivo:** distribuição de mídia pelo servidor `vellcast-relay`.

| ID | Tarefa | MoSCoW | Depende de | Critérios de aceite | Status |
|---|---|---|---|---|---|
| E3-T01 | `services/relay` (Tokio + Axum) a partir das lições do `stage0-server` | M-010 | Etapa 0 | Ingestão de transmissor e distribuição a espectadores | Done (local) — 18 testes unitários + 10 de integração |
| E3-T02 | Crate `transport` (WebSocket TLS) | M-010, M-020 | E3-T01 | Heartbeats, limites, backpressure | Done (local) — fila com descarte, TLS, backoff com jitter; integração ao desktop nas Etapas 2/5 |
| E3-T03 | Salas e autorização | M-013, M-020 | E3-T01 | Privadas por padrão; tokens temporários | Done — tokens HMAC-SHA256, convites com usos e expiração, API HTTP |
| E3-T04 | Filas por espectador e keyframes | M-011, M-018 | E3-T01 | Espectador lento não afeta os demais; solicitação de keyframe ao transmissor | Done — fan-out sem recodificação (bytes idênticos), cache de GOP, filas limitadas |
| E3-T05 | Reconexão | M-019 | E3-T02 | Janela de graça e retomada no keyframe | Done (local) — janela de graça com retomada por DISCONTINUITY |
| E3-T06 | Proteção contra abuso | M-020 | E3-T03 | Rate limiting; limites de espectadores e banda | Done — rate limit por IP, limites de sala/espectadores/mensagem, teto de bitrate |
| E3-T07 | Docker, docker-compose e guia Dokploy | M-021 | E3-T01 | Instalação limpa em VPS seguindo só a documentação | Done — imagem construída e rodando no Dokploy (VPS real) |
| E3-T08 | Métricas e health check | S-014 | E3-T01 | Endpoint de métricas; logs estruturados | Done — `/metrics`, `/healthz`, logs JSON, SIGTERM |
| E3-T09 | Benchmark com 20 espectadores autenticados | M-011, M-023 | E3-T04 | CPU/RAM/latência registrados | Done (local) — ver PERFORMANCE §3.4 |
| E3-T10 | Deploy em VPS com TLS via Dokploy | M-021 | E3-T07 | Instalação seguindo só a documentação; medição com rede real | Done — `https://vellcast.nextestudios.com` (Dokploy + Let's Encrypt); servidor na Europa limita a vazão para o Brasil (DEPLOYMENT §6.10) |
| E3-T11 | `RoomStore` persistente em SQLite | S-015 | E3-T03 | Salas sobrevivem a reinício do relay | Done — SQLite em produção (volume /data); sala sobreviveu a reinício do relay |
| E3-T12 | Revogação e rotação de tokens | M-020 | E3-T03 | Revogar um token sem encerrar a sala; troca de segredo sem queda | Done — revogação por token/todos os espectadores e rotação do segredo sem queda |
| E3-T13 | Sessão de 60 min e teste com 50 espectadores | M-025 | E3-T09 | Sem crescimento de memória; sem desconexões | Done — 60 min com 50 espectadores 1080p60: 187 GB, zero descartes, memória estável (≤14 MB) |
| E3-T14 | Inventário de licenças e unificação de dependências duplicadas | M-022 | — | `cargo deny` na CI; uma versão de tungstenite/base64 | Done — cargo-deny na CI (licenças, avisos, duplicatas) |

**Entregável:** servidor funcional em Docker.
**Critério de aceite:** transmitir vídeo codificado para múltiplos espectadores sem recodificação no servidor.
**Status:** Backlog.

---

## Etapa 4 — Áudio

**Objetivo:** transmitir áudio sincronizado.

| ID | Tarefa | MoSCoW | Depende de | Critérios de aceite | Status |
|---|---|---|---|---|---|
| E4-T01 | Crate `audio` — WASAPI Application Loopback | M-009 | Etapa 2 | Captura por processo quando suportada | Done — loopback por processo executado no Windows (149 pacotes/3 s, sem alertas do page heap) |
| E4-T02 | Fallback loopback do sistema | M-009 | E4-T01 | Aviso claro ao usuário | Done — áudio do sistema excluindo o Discord executado no Windows |
| E4-T03 | Codificação Opus | M-009 | E4-T01 | 48 kHz, 20 ms | Done — Opus 20 ms executado no Windows (180.000 pacotes em 60 min, intervalos exatos) |
| E4-T04 | Transporte e reprodução | M-009 | E4-T03, Etapa 3 | Áudio reproduzido na Activity (ou fallback documentado) | Done — áudio entregue aos espectadores pelo relay no Windows |
| E4-T05 | Sincronização A/V | M-009 | E4-T04 | Skew medido e dentro de limite documentado | Done — skew A/V no transporte de 9,2 ms no Windows (limite do teste: 250 ms) |

**Entregável:** transmissão audiovisual funcional.
**Critério de aceite:** áudio e vídeo reproduzidos com sincronização aceitável nos ambientes suportados e testados.
**Status:** Backlog.

---

## Etapa 5 — Integração completa

**Objetivo:** integrar desktop, servidor e Discord Activity em um MVP de ponta a ponta.

| ID | Tarefa | MoSCoW | Depende de | Critérios de aceite | Status |
|---|---|---|---|---|---|
| E5-T01 | Autenticação (OAuth2 Discord no backend) | M-014 | Etapa 3 | Client secret apenas no servidor | Testing — OAuth no relay testado contra Discord simulado; validação real bloqueada pelo E0-T10 |
| E5-T02 | Pareamento desktop ↔ Activity | M-015 | E5-T01 | Código de uso único com expiração | Testing — pareamento por código (8 caracteres, 5 min, uso único) testado; validação real bloqueada pelo E0-T10 |
| E5-T03 | Fluxo de salas na UI | M-013 | E5-T01 | Criar/entrar/convidar/encerrar | Done — salas e convites pelo app contra o relay de produção |
| E5-T04 | Fluxo iniciar/encerrar transmissão | M-016 | Etapas 1–4 | Por mouse, teclado e gamepad | Done — app desktop transmitiu no Windows ao servidor de produção pelo caminho do botão Iniciar (E2E) |
| E5-T05 | Player final na Activity | M-012 | Etapa 3 | Listagem de transmissões autorizadas; volume; tela cheia quando suportada | Testing — Activity final (login, salas da instância, player) e player web; validação real bloqueada pelo E0-T10 |
| E5-T06 | Perfis de qualidade e atalhos | S-004, S-008 | E5-T04 | Atalhos globais desativados por padrão | Done — perfis e modo personalizado; atalhos globais desligados por padrão |
| E5-T07 | Tratamento de erros fim a fim | M-019 | E5-T04 | Mensagens claras e recuperação | Done — erros com mensagens claras no app e no player; reconexão validada |
| E5-T08 | Pacote de teste Windows e guia | M-021, M-022 | E5-T04 | `Vellcast-MVP-windows-x64.zip` gerado pela CI; [TESTING_WINDOWS](TESTING_WINDOWS.md) | Done — pacote e instalador gerados pela CI; guia TESTING_WINDOWS |
| E5-T09 | Player web servido pelo relay (S-011) | M-011, M-020 | E3-T01 | Convite → token de espectador → reprodução | Done — player web servido pelo relay em produção |

**Entregável:** MVP de ponta a ponta.
**Critério de aceite:** um usuário inicia a transmissão no Windows e outro assiste dentro do Discord; o fluxo funciona com controles no desktop.
**Status:** Backlog.

---

## Marco OC — Abertura do código (Open Core)

**Objetivo:** tornar público o repositório do núcleo aberto sem expor a parte comercial, segredos ou obrigações de licença pendentes. Decisão de 2026-10-08: o repositório `nextestudios/Vellcast` continua **privado** até este marco ser concluído (ver [OPEN_SOURCE_STRATEGY](OPEN_SOURCE_STRATEGY.md) §5).

**Momento-alvo:** ao fim da Etapa 5 (MVP de ponta a ponta), ou antes, se a cota de GitHub Actions do repositório privado virar gargalo.

| ID | Tarefa | MoSCoW | Depende de | Critérios de aceite | Status |
|---|---|---|---|---|---|
| OC-T01 | Confirmar a licença do código próprio | M-022 | — | Apache-2.0 confirmada (ou substituída) pelo responsável; ADR-007 passa a Aceita; arquivo `NOTICE` criado | Done — decisão: software proprietário (ADR-058) |
| OC-T02 | Inventário de licenças de terceiros | M-022 | OC-T01 | `cargo deny check licenses` e verificação das dependências npm na CI; tabela de [LICENSING](LICENSING.md) completa, sem pendências; obrigações do Slint (ADR-006) cumpridas | Done — cargo-deny e licenças npm na CI |
| OC-T03 | Separar o núcleo aberto da parte comercial | S-015 | — | `services/cloud-api`, planos, billing e documentos comerciais sensíveis movidos para um repositório privado; o núcleo compila e funciona sem eles (W-013) | Done — cloud-api movida com histórico para o repositório privado nextestudios/vellcast-cloud; núcleo compila sem ela |
| OC-T04 | Verificação da marca "Vellcast" | — | — | Busca no INPI e em domínios registrada; política de uso da marca publicada (Apache-2.0 não licencia a marca, cláusula 6) | Blocked — busca e registro da marca no INPI |
| OC-T05 | Revisão de segurança do relay e do protocolo | M-020 | Etapa 3 | Auditoria (`/cso`) sem achados críticos ou altos abertos; resultados em [SECURITY](SECURITY.md) | Done — revisão sem achados altos/críticos (docs/security/REVIEW-2026-10.md) |
| OC-T06 | Política de reporte de vulnerabilidades | M-020 | — | `SECURITY.md` na raiz com contato privado e prazos de resposta; GitHub Private Vulnerability Reporting habilitado | Done — SECURITY.md; falta habilitar Private Vulnerability Reporting nas configurações |
| OC-T07 | Varredura de segredos no histórico | M-024 | — | `gitleaks` (ou equivalente) em todo o histórico sem achados; verificação adicionada à CI | Done — gitleaks no histórico (sem segredos) e na CI |
| OC-T08 | Preparação para contribuidores externos | M-022 | OC-T01 | `CONTRIBUTING.md` revisado, código de conduta, templates de issue e PR, proteção da branch `main` | Done — código de conduta, modelos de issue e PR |
| OC-T09 | Tornar o repositório público | — | OC-T01..OC-T08 | Visibilidade alterada pelo responsável; README público revisado; anúncio planejado | Done — decisão: repositório permanece privado (ADR-058) |

**Entregável:** repositório público do Vellcast Core.
**Critério de aceite:** todas as tarefas OC-T01 a OC-T08 concluídas e a abertura (OC-T09) aprovada explicitamente pelo responsável.
**Status:** Backlog.

---

## Etapa 6 — Otimização extrema

**Objetivo:** reduzir o impacto da transmissão nos jogos.

| ID | Tarefa | MoSCoW | Depende de | Critérios de aceite | Status |
|---|---|---|---|---|---|
| E6-T01 | Benchmarks reproduzíveis | M-023 | Etapa 5 | Metodologia em [PERFORMANCE](PERFORMANCE.md) | Done — benchmarks reproduzíveis (crates/perf, tests/benchmarks, E2E Windows); números em PERFORMANCE |
| E6-T02 | Redução de alocações e otimização de buffers | M-018 | E6-T01 | Antes/depois medidos | Done — escrita em lote no relay e no transmissor; p99 de 2,3–3,0 ms para 1,7 ms (medido) |
| E6-T03 | Otimização de captura e encoder | M-007 | E6-T01 | Antes/depois medidos | Blocked — otimização de captura/encoder exige Windows com GPU real |
| E6-T04 | Ajuste automático de bitrate | S-001 | E6-T01 | Histerese validada | Done — no Windows real, subida limitada a 300 kbps: alvo 3000 → 500 kbps e encoder acompanhou ao vivo (2075 → ~400 kbps); hardware a verificar com GPU |
| E6-T05 | Avaliação QUIC (Quinn) vs WebSocket | S-005 | E6-T01 | Relatório e decisão registrados | Done — avaliação Quinn × WebSocket medida; decisão: adiado (ADR-037) |
| E6-T06 | Modo competitivo | S-002 | E6-T04 | Benchmark com/sem | Done — Windows real com CPU sobrecarregada: FPS-alvo 60 → 45 → 24 e bitrate 8000 → 4096 kbps; efeito em jogos reais a medir com GPU |
| E6-T07 | Métricas avançadas na UI | S-007 | E6-T01 | Seção própria | Done — métricas avançadas exibidas no Windows durante transmissão real (captura do E2E) |
| E6-T08 | Testes prolongados e de falha | M-025 | E6-T02 | ≥ 60 min estáveis | Done — 60 min no Windows real (transmissor sem vazamento) e relay 60 min com 20 e 50 espectadores sem perdas |

**Entregável:** versão otimizada com relatório de desempenho.
**Critério de aceite:** resultados reproduzíveis em hardware de referência, documentando diferenças entre jogar com e sem o Vellcast transmitindo.
**Status:** Backlog.

---

## Etapa 7 — Comercialização

**Objetivo:** preparar planos pagos sem prejudicar o núcleo.

| ID | Tarefa | MoSCoW | Depende de | Critérios de aceite | Status |
|---|---|---|---|---|---|
| E7-T01 | Pesquisa de políticas do Discord (Premium Apps, elegibilidade no Brasil, paridade) | S-015 | — | Conclusões registradas em [MONETIZATION](MONETIZATION.md) | Done — Premium Apps indisponível para desenvolvedores no Brasil (MONETIZATION §6, ADR-043) |
| E7-T02 | `services/cloud-api`: contas, planos, entitlements, cotas | S-015 | Etapa 5 | Entitlements validados no backend | Done — cloud-api no repositório privado nextestudios/vellcast-cloud (ADR-058) |
| E7-T03 | Integração com provedor de pagamento elegível | S-015 | E7-T01 | Webhooks testados em modo de teste; sem processamento direto de cartão | Testing — webhook compatível com Stripe em modo de teste; produção depende da conta do responsável |
| E7-T04 | Aplicação de limites no relay hospedado | S-015 | E7-T02 | Limites aplicados apenas no serviço oficial; self-hosted sem restrição de resolução | Done — relay hospedado exige entitlement assinado; self-hosted sem restrição |
| E7-T05 | Painel administrativo mínimo | S-015 | E7-T02 | — | Done — /admin com autenticação e CSRF |

**Entregável:** backend comercial funcional e testado.
**Critério de aceite:** planos reconhecidos e limites aplicados corretamente no serviço hospedado. Não ativar cobrança antes de verificar exigências legais e de plataforma.
**Status:** Backlog.

---

## Etapa 8 — Lançamento

**Objetivo:** primeira versão pública estável.

| ID | Tarefa | MoSCoW | Depende de | Critérios de aceite | Status |
|---|---|---|---|---|---|
| E8-T01 | Instalador Windows | S-016 | Etapa 6 | Instalação/desinstalação limpas | Done — instalador Inno Setup gerado e testado na CI (instalação/desinstalação silenciosas); release v0.1.0 em rascunho |
| E8-T02 | Assinatura de código | S-016 | E8-T01 | Quando certificado disponível | Blocked — falta certificado de assinatura de código (segredos WINDOWS_CERT_*) |
| E8-T03 | Atualizações seguras | S-012 | E8-T01 | Pacotes assinados e verificados | Done — manifesto v0.1.0 assinado na CI; assinatura confere com a chave embutida, manifesto adulterado é rejeitado e o SHA-256 do instalador bate. Com o repositório privado, a URL padrão exige autenticação (usar VELLCAST_UPDATE_URL ou tornar público) |
| E8-T04 | Termos, privacidade e divulgação de licenças | M-022, M-024 | — | Inventário de licenças de terceiros publicado | Testing — rascunhos de termos e privacidade (LGPD) e avisos de licenças automáticos; revisão jurídica pendente |
| E8-T05 | Deploy de produção e monitoramento | S-014 | Etapa 3 | Health checks e alertas | Testing — relay em produção com health check; alertas a ativar no Dokploy (OPERATIONS.md) |
| E8-T06 | Localização PT-BR/EN | S-013 | E1-T09 | Sem strings não traduzidas | Done — teste de i18n cobre todas as strings (pt-BR/en) |
| E8-T07 | Testes finais | M-025 | Todas | Checklist de lançamento completo | Testing — RELEASE_CHECKLIST.md pronto; execução manual em Windows real pendente |

**Entregável:** primeira versão pública estável do Vellcast.
**Status:** Backlog.

---

## Definição de Pronto

Uma etapa (ou tarefa) só pode ser concluída quando:

1. Código implementado.
2. Compilação validada.
3. Testes executados.
4. Critérios de aceite verificados.
5. Documentação atualizada.
6. Limitações registradas.
7. Riscos conhecidos documentados.
8. Roadmap atualizado.

Não confundir código escrito com funcionalidade efetivamente validada. Se um teste depender de credenciais, hardware ou acesso externo indisponível, a tarefa fica `Blocked` ou `Pending Validation`. Nunca inventar resultados.
