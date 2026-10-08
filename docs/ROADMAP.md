# Roadmap — Vellcast

> Etapas, tarefas, dependências, entregáveis e status. Prioridades em [MOSCOW](MOSCOW.md); riscos em [RISKS](RISKS.md).
> Status permitidos: `Backlog`, `Ready`, `In Progress`, `Blocked`, `Testing`, `Done`. `Pending Validation` indica código escrito cuja validação depende de hardware, credenciais ou acesso externo indisponível.

Última revisão: 2026-10-08.

## Visão geral

| Etapa | Nome | Prioridade | Depende de | Status |
|---|---|---|---|---|
| 0 | Validação Discord | Must | — | Testing — validação local concluída; E0-T10 adiada ([ADR-013](DECISIONS.md)) |
| 1 | Desktop e controles | Must | 0 (local) | Testing — falta Windows real e controle físico |
| 2 | Captura e encoder | Must | 0 (local) | Testing — compila no Windows; execução real pendente |
| 3 | Media relay | Must | 0 (local) | Testing — falta Docker e VPS com TLS |
| 4 | Áudio | Must | 2, 3 | Testing — compila no Windows; execução real pendente |
| 5 | Integração completa | Must | 1, 2, 3, 4 (E0-T10 para a Activity) | Testing — MVP Windows + player web pronto para teste |
| OC | Marco: abertura do código (Open Core) | Should | 5 | Backlog |
| 6 | Otimização extrema | Must | 5 | Backlog |
| 7 | Comercialização | Should (MVP) / Must (Premium) | 5 | Backlog |
| 8 | Lançamento | Must (lançamento) | 6, 7 | Backlog |

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
| E0-T05 | Activity mínima | M-012 | E0-T04 | React/TS/Vite; Embedded App SDK `ready()` quando dentro do Discord e modo standalone fora dele; `VideoDecoder` (Annex B, codec string derivada do SPS) e `AudioDecoder` Opus; detecção via `isConfigSupported` | Testing — reprodução validada fora do Discord; `ready()` pendente de app Discord |
| E0-T06 | Métricas no player | M-023 | E0-T05 | Exibe FPS decodificado, frames descartados, bitrate recebido, latência servidor→decodificação (com estimativa de offset de relógio por ping/pong) e A/V skew | Testing — métricas validadas localmente |
| E0-T07 | Reconexão | M-019 | E0-T05 | Backoff exponencial com jitter; retomada no próximo keyframe; tempo de recuperação medido ao reiniciar o servidor | Testing — reconexão validada localmente após reinício do servidor |
| E0-T08 | Verificação da Gamepad API | M-003 | E0-T05 | Relatório de disponibilidade de `navigator.getGamepads()` e eventos de conexão, exibido no player | Testing — API detectada; falta controle físico e o iframe do Discord |
| E0-T09 | Teste em navegador local | M-012 | E0-T04..T08 | 720p30 e 1080p60 reproduzidos em Chrome/Edge locais com métricas registradas | Testing — validado em Chromium headless; falta Chrome/Edge com interface e GPU |
| E0-T10 | Teste em Activity real | M-012 | E0-T09 | Activity executada no Discord Desktop e Web (mobile quando aplicável) via URL Mapping; métricas registradas | Backlog — adiada por decisão do responsável ([ADR-013](DECISIONS.md)) |
| E0-T11 | Relatório de resultados | M-022, M-023 | E0-T09, E0-T10 | Resultados de vídeo, áudio, FPS, latência, reconexão e gamepad registrados separadamente em [PERFORMANCE](PERFORMANCE.md), com hardware, sistema, rede e metodologia | In Progress — resultados locais em PERFORMANCE.md |
**Entregável:** protótipo reproduzível dentro do Discord.

**Critério de aceite da etapa:** vídeo reproduzido dentro de uma Activity real, com métricas documentadas; áudio, FPS, latência e suporte a controle registrados separadamente. Se algum requisito fundamental falhar (por exemplo, `VideoDecoder` H.264 indisponível ou throughput insuficiente no proxy), documentar o bloqueio em [DECISIONS](DECISIONS.md) e avaliar alternativas antes de avançar.

**Status:** Testing. Validação local concluída; E0-T10 adiada ([ADR-013](DECISIONS.md)).

---

## Etapa 1 — Desktop e controles

**Objetivo:** aplicativo Windows extremamente leve, navegável por controle.

| ID | Tarefa | MoSCoW | Depende de | Critérios de aceite | Status |
|---|---|---|---|---|---|
| E1-T01 | Decisão de licença do Slint | M-001 | — | Opção registrada em [LICENSING](LICENSING.md) e [DECISIONS](DECISIONS.md) | Done — Royalty-free 2.0 (ADR-006) |
| E1-T02 | Esqueleto `apps/desktop` (Rust + Slint) | M-001 | E1-T01 | Build Release na CI Windows | Testing — compila para Windows; falta build Release na CI Windows e execução real |
| E1-T03 | Design system em Slint | M-002 | E1-T02 | Tokens de cor, tipografia, espaçamento e foco conforme [UX_GUIDELINES](UX_GUIDELINES.md) | Testing — revisão visual no Windows pendente |
| E1-T04 | Navegação lateral e telas (Home, Transmitir, Salas, Configurações) | M-002 | E1-T03 | Telas navegáveis; nenhuma tela simulada apresentada como funcional | Testing — 8 telas; etapas futuras com estado honesto ("disponível na Etapa N") |
| E1-T05 | Crate `controller` (GilRs + XInput quando necessário) | M-003, S-010 | E1-T02 | Mapeamento da seção 13; deadzone; repetição controlada; conexão/desconexão | Testing — 16 testes de lógica; controle físico não testado |
| E1-T06 | Sistema de foco e modais | M-003 | E1-T04, E1-T05 | Foco sempre visível; modais navegáveis | Testing — fluxo completo validado com ações simuladas (ADR-015) |
| E1-T07 | Teclado virtual | M-004 | E1-T06 | Requisitos da seção 14 | Testing — validado com ações simuladas; controle físico pendente |
| E1-T08 | Modo bandeja | M-017 | E1-T02 | Sem renderização quando minimizado | Testing — sem redesenho oculto (20 MB, ~0,1% CPU no macOS); clique/menu e Windows pendentes |
| E1-T09 | Preparação de i18n | S-013 | E1-T04 | Strings externalizadas | Done — pt-BR/en com troca em tempo real; teste falha se faltar tradução (ADR-017) |
| E1-T10 | Medição de CPU/RAM em Release | M-023 | E1-T08 | Ocioso e minimizado medidos em Windows real | Blocked — requer máquina Windows; números do macOS em PERFORMANCE |

**Entregável:** aplicativo navegável por controle.
**Critério de aceite:** todas as operações implementadas nesta etapa acessíveis por controle; CPU e RAM medidos em Release.
**Status:** Backlog.

---

## Etapa 2 — Captura e encoder

**Objetivo:** capturar e codificar vídeo nativamente, mantendo frames na GPU.

| ID | Tarefa | MoSCoW | Depende de | Critérios de aceite | Status |
|---|---|---|---|---|---|
| E2-T01 | Crate `capture` — WGC de janela | M-005 | Etapa 0 | Seleção explícita; resize; fechamento; perda de dispositivo | Testing — enumeração de janelas e monitores executada no Windows da CI; captura real pendente |
| E2-T02 | WGC de monitor | M-006 | E2-T01 | Multi-monitor; mudança de resolução | Testing — compila no Windows; execução real pendente |
| E2-T03 | Fallback DXGI Desktop Duplication (se necessário) | M-006 | E2-T02 | Somente se justificado por teste | Backlog — só se o WGC falhar nos testes reais |
| E2-T04 | Crate `encoder` — detecção de encoders (MF) | M-007, S-006 | E2-T01 | Lista NVENC/AMF/QSV disponíveis | Testing — enumeração executada no Windows da CI (H264 Encoder MFT encontrado); GPUs reais pendentes |
| E2-T05 | Pipeline D3D11 → encoder H.264 sem cópia para CPU | M-007, M-018 | E2-T04 | Baixa latência; sem B-frames; keyframe sob demanda; fila limitada | Testing — encoder executado no Windows da CI (software + conversão na CPU, ADR-031): SPS+IDR e keyframe sob demanda OK; caminho de GPU pendente |
| E2-T06 | Configuração de resolução/FPS/bitrate | M-016 | E2-T05 | Reconfiguração ou reinício controlado | Testing — definidos ao iniciar; mudar exige reiniciar a transmissão |
| E2-T07 | Detecção de jogos | S-003 | E2-T01 | Heurística não invasiva | Testing — heurística testada; enumeração real pendente |
| E2-T08 | Medição 1080p60 | M-008, M-023 | E2-T05 | Consumo e limitações registrados por GPU | Blocked — requer máquina Windows com GPU |
| E2-T09 | Pipeline do `vellcast-core` (captura → encoder → relay, eventos, métricas, recuperação de dispositivo) | M-007, M-018 | E2-T05, E3-T02 | Sessão inicia, transmite e encerra pelo app | Testing — compila no Windows; execução real pendente |
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
| E3-T07 | Docker, docker-compose e guia Dokploy | M-021 | E3-T01 | Instalação limpa em VPS seguindo só a documentação | Testing — Dockerfile e compose escritos; build Docker não executado |
| E3-T08 | Métricas e health check | S-014 | E3-T01 | Endpoint de métricas; logs estruturados | Done — `/metrics`, `/healthz`, logs JSON, SIGTERM |
| E3-T09 | Benchmark com 20 espectadores autenticados | M-011, M-023 | E3-T04 | CPU/RAM/latência registrados | Done (local) — ver PERFORMANCE §3.4 |
| E3-T10 | Deploy em VPS com TLS via Dokploy | M-021 | E3-T07 | Instalação seguindo só a documentação; medição com rede real | Backlog |
| E3-T11 | `RoomStore` persistente em SQLite | S-015 | E3-T03 | Salas sobrevivem a reinício do relay | Backlog |
| E3-T12 | Revogação e rotação de tokens | M-020 | E3-T03 | Revogar um token sem encerrar a sala; troca de segredo sem queda | Backlog |
| E3-T13 | Sessão de 60 min e teste com 50 espectadores | M-025 | E3-T09 | Sem crescimento de memória; sem desconexões | Backlog |
| E3-T14 | Inventário de licenças e unificação de dependências duplicadas | M-022 | — | `cargo deny` na CI; uma versão de tungstenite/base64 | Backlog |

**Entregável:** servidor funcional em Docker.
**Critério de aceite:** transmitir vídeo codificado para múltiplos espectadores sem recodificação no servidor.
**Status:** Backlog.

---

## Etapa 4 — Áudio

**Objetivo:** transmitir áudio sincronizado.

| ID | Tarefa | MoSCoW | Depende de | Critérios de aceite | Status |
|---|---|---|---|---|---|
| E4-T01 | Crate `audio` — WASAPI Application Loopback | M-009 | Etapa 2 | Captura por processo quando suportada | Testing — compila no Windows; execução real pendente |
| E4-T02 | Fallback loopback do sistema | M-009 | E4-T01 | Aviso claro ao usuário | Testing — exclui a árvore do Discord; fallback clássico com aviso |
| E4-T03 | Codificação Opus | M-009 | E4-T01 | 48 kHz, 20 ms | Testing — libopus compila na CI Windows |
| E4-T04 | Transporte e reprodução | M-009 | E4-T03, Etapa 3 | Áudio reproduzido na Activity (ou fallback documentado) | Testing — áudio integrado ao pipeline e ao player; execução real pendente |
| E4-T05 | Sincronização A/V | M-009 | E4-T04 | Skew medido e dentro de limite documentado | In Progress — PTS no relógio QPC comum; skew a medir no Windows |

**Entregável:** transmissão audiovisual funcional.
**Critério de aceite:** áudio e vídeo reproduzidos com sincronização aceitável nos ambientes suportados e testados.
**Status:** Backlog.

---

## Etapa 5 — Integração completa

**Objetivo:** integrar desktop, servidor e Discord Activity em um MVP de ponta a ponta.

| ID | Tarefa | MoSCoW | Depende de | Critérios de aceite | Status |
|---|---|---|---|---|---|
| E5-T01 | Autenticação (OAuth2 Discord no backend) | M-014 | Etapa 3 | Client secret apenas no servidor | Backlog — adiado (ADR-013); no MVP o acesso é por link de convite |
| E5-T02 | Pareamento desktop ↔ Activity | M-015 | E5-T01 | Código de uso único com expiração | Backlog — adiado (ADR-013); o link de convite substitui no MVP |
| E5-T03 | Fluxo de salas na UI | M-013 | E5-T01 | Criar/entrar/convidar/encerrar | Testing — validado no macOS contra relay real; Windows pendente |
| E5-T04 | Fluxo iniciar/encerrar transmissão | M-016 | Etapas 1–4 | Por mouse, teclado e gamepad | Testing — depende do pipeline no Windows |
| E5-T05 | Player final na Activity | M-012 | Etapa 3 | Listagem de transmissões autorizadas; volume; tela cheia quando suportada | Testing — player web (/watch) validado a 60 fps; Activity no E0-T10 |
| E5-T06 | Perfis de qualidade e atalhos | S-004, S-008 | E5-T04 | Atalhos globais desativados por padrão | Testing — perfis e modo personalizado; atalhos globais desligados |
| E5-T07 | Tratamento de erros fim a fim | M-019 | E5-T04 | Mensagens claras e recuperação | Testing — mensagens claras no app e no player; Windows pendente |
| E5-T08 | Pacote de teste Windows e guia | M-021, M-022 | E5-T04 | `Vellcast-MVP-windows-x64.zip` gerado pela CI; [TESTING_WINDOWS](TESTING_WINDOWS.md) | Testing — gerado pela CI; teste do usuário pendente |
| E5-T09 | Player web servido pelo relay (S-011) | M-011, M-020 | E3-T01 | Convite → token de espectador → reprodução | Testing — validado no macOS (60 fps, p95 3 ms) |

**Entregável:** MVP de ponta a ponta.
**Critério de aceite:** um usuário inicia a transmissão no Windows e outro assiste dentro do Discord; o fluxo funciona com controles no desktop.
**Status:** Backlog.

---

## Marco OC — Abertura do código (Open Core)

**Objetivo:** tornar público o repositório do núcleo aberto sem expor a parte comercial, segredos ou obrigações de licença pendentes. Decisão de 2026-10-08: o repositório `nextestudios/Vellcast` continua **privado** até este marco ser concluído (ver [OPEN_SOURCE_STRATEGY](OPEN_SOURCE_STRATEGY.md) §5).

**Momento-alvo:** ao fim da Etapa 5 (MVP de ponta a ponta), ou antes, se a cota de GitHub Actions do repositório privado virar gargalo.

| ID | Tarefa | MoSCoW | Depende de | Critérios de aceite | Status |
|---|---|---|---|---|---|
| OC-T01 | Confirmar a licença do código próprio | M-022 | — | Apache-2.0 confirmada (ou substituída) pelo responsável; ADR-007 passa a Aceita; arquivo `NOTICE` criado | Backlog |
| OC-T02 | Inventário de licenças de terceiros | M-022 | OC-T01 | `cargo deny check licenses` e verificação das dependências npm na CI; tabela de [LICENSING](LICENSING.md) completa, sem pendências; obrigações do Slint (ADR-006) cumpridas | Backlog |
| OC-T03 | Separar o núcleo aberto da parte comercial | S-015 | — | `services/cloud-api`, planos, billing e documentos comerciais sensíveis movidos para um repositório privado; o núcleo compila e funciona sem eles (W-013) | Backlog |
| OC-T04 | Verificação da marca "Vellcast" | — | — | Busca no INPI e em domínios registrada; política de uso da marca publicada (Apache-2.0 não licencia a marca, cláusula 6) | Backlog |
| OC-T05 | Revisão de segurança do relay e do protocolo | M-020 | Etapa 3 | Auditoria (`/cso`) sem achados críticos ou altos abertos; resultados em [SECURITY](SECURITY.md) | Backlog |
| OC-T06 | Política de reporte de vulnerabilidades | M-020 | — | `SECURITY.md` na raiz com contato privado e prazos de resposta; GitHub Private Vulnerability Reporting habilitado | Backlog |
| OC-T07 | Varredura de segredos no histórico | M-024 | — | `gitleaks` (ou equivalente) em todo o histórico sem achados; verificação adicionada à CI | Backlog |
| OC-T08 | Preparação para contribuidores externos | M-022 | OC-T01 | `CONTRIBUTING.md` revisado, código de conduta, templates de issue e PR, proteção da branch `main` | Backlog |
| OC-T09 | Tornar o repositório público | — | OC-T01..OC-T08 | Visibilidade alterada pelo responsável; README público revisado; anúncio planejado | Backlog |

**Entregável:** repositório público do Vellcast Core.
**Critério de aceite:** todas as tarefas OC-T01 a OC-T08 concluídas e a abertura (OC-T09) aprovada explicitamente pelo responsável.
**Status:** Backlog.

---

## Etapa 6 — Otimização extrema

**Objetivo:** reduzir o impacto da transmissão nos jogos.

| ID | Tarefa | MoSCoW | Depende de | Critérios de aceite | Status |
|---|---|---|---|---|---|
| E6-T01 | Benchmarks reproduzíveis | M-023 | Etapa 5 | Metodologia em [PERFORMANCE](PERFORMANCE.md) | Backlog |
| E6-T02 | Redução de alocações e otimização de buffers | M-018 | E6-T01 | Antes/depois medidos | Backlog |
| E6-T03 | Otimização de captura e encoder | M-007 | E6-T01 | Antes/depois medidos | Backlog |
| E6-T04 | Ajuste automático de bitrate | S-001 | E6-T01 | Histerese validada | Backlog |
| E6-T05 | Avaliação QUIC (Quinn) vs WebSocket | S-005 | E6-T01 | Relatório e decisão registrados | Backlog |
| E6-T06 | Modo competitivo | S-002 | E6-T04 | Benchmark com/sem | Backlog |
| E6-T07 | Métricas avançadas na UI | S-007 | E6-T01 | Seção própria | Backlog |
| E6-T08 | Testes prolongados e de falha | M-025 | E6-T02 | ≥ 60 min estáveis | Backlog |

**Entregável:** versão otimizada com relatório de desempenho.
**Critério de aceite:** resultados reproduzíveis em hardware de referência, documentando diferenças entre jogar com e sem o Vellcast transmitindo.
**Status:** Backlog.

---

## Etapa 7 — Comercialização

**Objetivo:** preparar planos pagos sem prejudicar o núcleo.

| ID | Tarefa | MoSCoW | Depende de | Critérios de aceite | Status |
|---|---|---|---|---|---|
| E7-T01 | Pesquisa de políticas do Discord (Premium Apps, elegibilidade no Brasil, paridade) | S-015 | — | Conclusões registradas em [MONETIZATION](MONETIZATION.md) | Backlog |
| E7-T02 | `services/cloud-api`: contas, planos, entitlements, cotas | S-015 | Etapa 5 | Entitlements validados no backend | Backlog |
| E7-T03 | Integração com provedor de pagamento elegível | S-015 | E7-T01 | Webhooks testados em modo de teste; sem processamento direto de cartão | Backlog |
| E7-T04 | Aplicação de limites no relay hospedado | S-015 | E7-T02 | Limites aplicados apenas no serviço oficial; self-hosted sem restrição de resolução | Backlog |
| E7-T05 | Painel administrativo mínimo | S-015 | E7-T02 | — | Backlog |

**Entregável:** backend comercial funcional e testado.
**Critério de aceite:** planos reconhecidos e limites aplicados corretamente no serviço hospedado. Não ativar cobrança antes de verificar exigências legais e de plataforma.
**Status:** Backlog.

---

## Etapa 8 — Lançamento

**Objetivo:** primeira versão pública estável.

| ID | Tarefa | MoSCoW | Depende de | Critérios de aceite | Status |
|---|---|---|---|---|---|
| E8-T01 | Instalador Windows | S-016 | Etapa 6 | Instalação/desinstalação limpas | Backlog |
| E8-T02 | Assinatura de código | S-016 | E8-T01 | Quando certificado disponível | Backlog |
| E8-T03 | Atualizações seguras | S-012 | E8-T01 | Pacotes assinados e verificados | Backlog |
| E8-T04 | Termos, privacidade e divulgação de licenças | M-022, M-024 | — | Inventário de licenças de terceiros publicado | Backlog |
| E8-T05 | Deploy de produção e monitoramento | S-014 | Etapa 3 | Health checks e alertas | Backlog |
| E8-T06 | Localização PT-BR/EN | S-013 | E1-T09 | Sem strings não traduzidas | Backlog |
| E8-T07 | Testes finais | M-025 | Todas | Checklist de lançamento completo | Backlog |

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
