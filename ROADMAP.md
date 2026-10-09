# Vellcast — Roadmap completo

Visão única de tudo o que será feito, da validação inicial ao lançamento, organizada pela metodologia **MoSCoW**.
Critérios de aceite, dependências e notas de cada tarefa estão em [`docs/ROADMAP.md`](docs/ROADMAP.md) (fonte da verdade); a classificação completa dos requisitos está em [`docs/MOSCOW.md`](docs/MOSCOW.md).

Atualizado em: **2026-10-09**

**Legenda:** ✅ concluído · 🧪 em teste · 🔨 em andamento · ⛔ bloqueado · ⏸️ adiado · ⬜ não iniciado
**MoSCoW:** **M** = Must Have · **S** = Should Have · **C** = Could Have · **W** = Won't Have (fora do MVP)

---

## 1. Visão geral

```mermaid
flowchart LR
    E0["Etapa 0<br/>Validação Discord"] --> E1["Etapa 1<br/>Desktop e controles"]
    E0 --> E2["Etapa 2<br/>Captura e encoder"]
    E0 --> E3["Etapa 3<br/>Media relay"]
    E2 --> E4["Etapa 4<br/>Áudio"]
    E3 --> E4
    E1 --> E5["Etapa 5<br/>Integração completa (MVP)"]
    E2 --> E5
    E3 --> E5
    E4 --> E5
    E5 --> OC["Marco OC<br/>Abertura do código"]
    E5 --> E6["Etapa 6<br/>Otimização extrema"]
    E5 --> E7["Etapa 7<br/>Comercialização"]
    E6 --> E8["Etapa 8<br/>Lançamento"]
    E7 --> E8
```

| Etapa | Nome | Prioridade | Ambiente necessário | Situação |
|---|---|---|---|---|
| 0 | Validação Discord | M | macOS + Discord | ⛔ código pronto (OAuth, pareamento, Activity); falta criar a aplicação no Discord |
| 1 | Desktop e controles | M | macOS (dev) + Windows (validação) | 🧪 implementada; falta Windows real e controle físico |
| 2 | Captura e encoder | M | **Windows com GPU** | 🧪 executada no Windows (CI, sem GPU); falta GPU real |
| 3 | Media relay | M | macOS/Linux | ✅ em produção em `vellcast.nextestudios.com`; 60 min estável |
| 4 | Áudio | M | **Windows** | 🧪 executada no Windows (CI); falta teste com jogo real |
| 5 | Integração completa — **MVP** | M | Windows (+ Discord para a Activity) | 🧪 E2E Windows passando; servidor de testes no ar; pronto para teste |
| OC | Abertura do código (Open Core) | S | — | 🧪 parte técnica pronta; decisões suas pendentes |
| 6 | Otimização extrema | M | Windows com GPU + jogos | 🧪 software pronto e medido; otimização com GPU real bloqueada |
| 7 | Comercialização | S no MVP / M no Premium | Linux (servidor) | 🧪 backend pronto; cobrança real depende de conta no provedor |
| 8 | Lançamento | M | Windows + produção | 🧪 instalador e atualizador prontos; certificado e revisão jurídica pendentes |

**Marcos:**
1. **MVP** (fim da Etapa 5): um usuário transmite um jogo do Windows e outro assiste dentro do Discord, com controle no desktop.
2. **Núcleo aberto** (Marco OC): repositório público do Vellcast Core.
3. **Lançamento público** (Etapa 8): primeira versão estável com instalador.

---

## 2. Etapas e tarefas

### Etapa 0 — Validação Discord · M
Comprovar que H.264 + Opus via WebSocket e WebCodecs funcionam dentro de uma Discord Activity.

- [x] ✅ E0-T01 — Estrutura do monorepositório
- [x] ✅ E0-T02 — Protocolo binário v1 (`crates/protocol`) com testes
- [x] ✅ E0-T03 — Geração de mídia sintética (720p30 e 1080p60, Opus)
- [x] ✅ E0-T04 — Servidor WebSocket de teste com distribuição para vários espectadores
- [ ] 🧪 E0-T05 — Activity mínima (React + Embedded App SDK + WebCodecs)
- [ ] 🧪 E0-T06 — Métricas no player (FPS, descartes, bitrate, latência, A/V skew)
- [ ] 🧪 E0-T07 — Reconexão automática
- [ ] 🧪 E0-T08 — Verificação da Gamepad API
- [ ] 🧪 E0-T09 — Teste em navegador local
- [ ] ⛔ E0-T10 — Teste em Activity real (Desktop, Web, Mobile) — código pronto; falta criar a aplicação no Discord ([guia](docs/DISCORD_INTEGRATION.md))
- [ ] 🔨 E0-T11 — Relatório de resultados

### Etapa 1 — Desktop e controles · M
Aplicativo Windows leve em Rust + Slint, 100% navegável por controle.

- [x] ✅ E1-T01 — Licença do Slint: Royalty-free 2.0
- [ ] 🧪 E1-T02 — Esqueleto `apps/desktop` (Rust + Slint) — compila para Windows; execução real pendente
- [ ] 🧪 E1-T03 — Design system em Slint
- [ ] 🧪 E1-T04 — Navegação lateral e 8 telas (Home, Transmitir, Salas, Ao vivo, Configurações, Conta, Planos, Sobre)
- [ ] 🧪 E1-T05 — Crate `controller` (GilRs) — falta teste com controle físico
- [ ] 🧪 E1-T06 — Sistema de foco e modais navegáveis
- [ ] 🧪 E1-T07 — Teclado virtual
- [ ] 🧪 E1-T08 — Modo bandeja sem renderização em segundo plano
- [x] ✅ E1-T09 — Preparação de i18n (PT-BR e EN, troca em tempo real)
- [ ] ⛔ E1-T10 — Medição de CPU/RAM em Release no Windows — requer máquina Windows (macOS: ~0,1% CPU, 20–33 MB)

### Etapa 2 — Captura e encoder · M
Capturar e codificar 1080p60 nativamente, mantendo os quadros na GPU.

- [ ] 🧪 E2-T01 — Captura de janela (Windows Graphics Capture)
- [ ] 🧪 E2-T02 — Captura de monitor
- [x] ✅ E2-T03 — Fallback DXGI: não necessário, o WGC funcionou no Windows real (ADR-057)
- [ ] 🧪 E2-T04 — Detecção de encoders (hardware primeiro, software como fallback)
- [ ] 🧪 E2-T05 — Pipeline Direct3D 11 → NV12 na GPU → H.264 sem cópia para a CPU
- [ ] 🧪 E2-T06 — Configuração de resolução, FPS e bitrate
- [ ] 🧪 E2-T07 — Detecção automática de jogos (S)
- [ ] ⛔ E2-T08 — Medição 1080p60 por GPU — requer máquina Windows
- [ ] 🧪 E2-T09 — Pipeline do `vellcast-core` (captura → encoder → relay)
- [x] ✅ E2-T10 — Ferramentas de validação manual (`capture_to_file`)

### Etapa 3 — Media relay · M
Servidor `vellcast-relay` que recebe uma transmissão e distribui para vários espectadores, sem recodificar.

- [x] ✅ E3-T01 — `services/relay` (Tokio + Axum)
- [x] ✅ E3-T02 — Crate `transport` (cliente do transmissor, WebSocket TLS)
- [x] ✅ E3-T03 — Salas privadas e autorização com tokens temporários
- [x] ✅ E3-T04 — Filas limitadas por espectador, cache de GOP e pedidos de keyframe
- [x] ✅ E3-T05 — Reconexão com janela de graça
- [x] ✅ E3-T06 — Proteção contra abuso (rate limiting, limites de espectadores e banda)
- [x] ✅ E3-T07 — Docker, docker-compose e guia Dokploy (imagem rodando no Dokploy)
- [x] ✅ E3-T08 — Métricas e health check
- [x] ✅ E3-T09 — Benchmark com 20 espectadores (local: 60 fps, p95 2,2 ms, 4,8% CPU, 9,7 MB)
- [x] ✅ E3-T10 — Deploy em VPS com TLS via Dokploy: `https://vellcast.nextestudios.com` (servidor na Europa limita a vazão para o Brasil)
- [x] ✅ E3-T11 — Salas persistentes em SQLite (em produção, sobrevivem a reinícios) (S)
- [x] ✅ E3-T12 — Revogação de tokens e rotação do segredo sem queda
- [ ] 🔨 E3-T13 — Sessão de 60 min com 50 espectadores (em execução)
- [x] ✅ E3-T14 — Inventário de licenças e duplicatas (`cargo deny` na CI)

### Etapa 4 — Áudio · M
Transmitir o áudio do jogo, sincronizado com o vídeo.

- [ ] 🧪 E4-T01 — Captura WASAPI por processo (Application Loopback)
- [ ] 🧪 E4-T02 — Fallback: áudio do sistema excluindo o Discord, com aviso ao usuário
- [ ] 🧪 E4-T03 — Codificação Opus (48 kHz, 20 ms)
- [ ] 🧪 E4-T04 — Transporte e reprodução
- [ ] 🔨 E4-T05 — Sincronização A/V (relógio comum pronto; medir no Windows)

### Etapa 5 — Integração completa (MVP) · M
Desktop, relay e Discord Activity funcionando juntos de ponta a ponta.

- [ ] 🧪 E5-T01 — Autenticação OAuth2 do Discord no relay (testada com Discord simulado)
- [ ] 🧪 E5-T02 — Pareamento desktop ↔ Activity por código de uso único
- [ ] 🧪 E5-T03 — Fluxo de salas na interface (criar, convidar, encerrar)
- [ ] 🧪 E5-T04 — Fluxo de iniciar e encerrar transmissão (mouse, teclado e controle)
- [ ] 🧪 E5-T05 — Player final: web em `/watch` e Activity (login, salas da instância, player)
- [ ] 🧪 E5-T06 — Perfis de qualidade e atalhos (S)
- [ ] 🧪 E5-T07 — Tratamento de erros de ponta a ponta
- [ ] 🧪 E5-T08 — Pacote de teste Windows (`Vellcast-MVP-windows-x64.zip`) e guia [`docs/TESTING_WINDOWS.md`](docs/TESTING_WINDOWS.md)
- [ ] 🧪 E5-T09 — Player web servido pelo relay (S-011)
- [ ] ⛔ Validação dentro do Discord: E0-T10

### Marco OC — Abertura do código (Open Core) · S
Repositório continua **privado** até concluir este marco (alvo: fim da Etapa 5).

- [ ] ⛔ OC-T01 — Confirmar a licença do código próprio (Apache-2.0) — decisão sua
- [x] ✅ OC-T02 — Inventário de licenças de terceiros (`cargo deny` + npm na CI)
- [ ] ⛔ OC-T03 — Separar a parte comercial em repositório privado — decisão sua
- [ ] ⛔ OC-T04 — Verificação da marca "Vellcast" (INPI) — ação sua
- [x] ✅ OC-T05 — Revisão de segurança (sem achados altos ou críticos)
- [x] ✅ OC-T06 — `SECURITY.md` (falta habilitar Private Vulnerability Reporting no GitHub)
- [x] ✅ OC-T07 — Varredura de segredos no histórico e na CI (gitleaks)
- [x] ✅ OC-T08 — Código de conduta e modelos de issue/PR
- [ ] ⛔ OC-T09 — Tornar o repositório público — aprovação sua

### Etapa 6 — Otimização extrema · M
Reduzir ao mínimo o impacto da transmissão nos jogos, com números reproduzíveis.

- [x] ✅ E6-T01 — Benchmarks reproduzíveis (`crates/perf`, `tests/benchmarks`, E2E Windows)
- [x] ✅ E6-T02 — Escrita em lote no relay e no transmissor (p99 2,3–3,0 ms → 1,7 ms)
- [ ] ⛔ E6-T03 — Otimização de captura e encoder — requer Windows com GPU real
- [ ] 🧪 E6-T04 — Ajuste automático de bitrate (S)
- [x] ✅ E6-T05 — Avaliação QUIC (Quinn) vs WebSocket: medida; QUIC adiado (ADR-037)
- [ ] 🧪 E6-T06 — Modo competitivo (S)
- [ ] 🧪 E6-T07 — Métricas avançadas na interface (S)
- [x] ✅ E6-T08 — Testes prolongados: 60 min no Windows real e relay 60 min com 20/50 espectadores

### Etapa 7 — Comercialização · S no MVP / M no Premium
Planos Free, Pro e Community sem prejudicar o núcleo gratuito (1080p60 continua grátis).

- [x] ✅ E7-T01 — Políticas do Discord: Premium Apps indisponível no Brasil (venda fora do Discord)
- [x] ✅ E7-T02 — `services/cloud-api`: contas, planos, assinaturas, entitlements e cotas
- [ ] 🧪 E7-T03 — Pagamentos (webhook compatível com Stripe, modo de teste) — produção depende da sua conta no provedor
- [x] ✅ E7-T04 — Limites aplicados só no relay hospedado (self-hosted sem restrição)
- [x] ✅ E7-T05 — Painel administrativo mínimo (`/admin`)

### Etapa 8 — Lançamento · M
Primeira versão pública estável.

- [ ] 🧪 E8-T01 — Instalador Windows (Inno Setup, teste de instalar/desinstalar na CI) (S)
- [ ] ⛔ E8-T02 — Assinatura de código — falta o certificado
- [ ] 🧪 E8-T03 — Atualizações seguras (manifesto assinado Ed25519 + SHA-256) (S)
- [ ] 🧪 E8-T04 — Termos, privacidade e licenças — rascunhos; revisão jurídica pendente
- [ ] 🧪 E8-T05 — Produção e monitoramento — alertas a ativar no Dokploy
- [x] ✅ E8-T06 — Localização PT-BR e EN (S)
- [ ] 🧪 E8-T07 — Testes finais — checklist pronto, execução em Windows real pendente

---

## 3. Depois do lançamento — Could Have

Só entram depois que o núcleo estiver estável.

| ID | Item | Observação |
|---|---|---|
| C-001 | Transmissão 1440p60 | Benefício Pro no serviço oficial |
| C-002 | Transmissão 4K60 | Benefício Pro no serviço oficial |
| C-003 | Codec AV1 | Depende de suporte de encoder e decoder |
| C-004 | Codec H.265 | Depende de suporte de decoder nos clientes |
| C-005 | Transmissão a 120 FPS | |
| C-006 | Temas personalizados | |
| C-007 | Salas públicas | Privadas continuam sendo o padrão |
| C-008 | Descoberta de transmissões | |
| C-009 | Gravação local | Nunca automática |
| C-010 | Picture-in-picture | |
| C-011 | Várias transmissões em uma sala | |
| C-012 | Gerenciamento avançado de comunidades | Plano Community |
| C-013 | Dashboard web administrativo | |
| C-014 | Servidores de mídia regionais | |
| C-015 | Aplicativo Linux | |
| C-016 | Aplicativo macOS | |
| C-017 | Player mobile independente | |
| C-018 | Sistema de plugins | Não pode expor APIs do Slint (ADR-006) |
| C-019 | Integrações adicionais | |
| C-020 | Modo P2P opcional fora do Discord | |

## 4. Fora do escopo do MVP — Won't Have

Electron (W-001) · navegador completo embutido no transmissor (W-002) · chat de voz próprio (W-003) · substituir o Discord (W-004) · WebRTC na Activity (W-005) · recodificação obrigatória no servidor (W-006) · captura escondida (W-007) · gravação automática (W-008) · animações pesadas (W-009) · mineração, publicidade invasiva ou processamento oculto (W-010) · injeção em processos de jogos (W-011) · interferência com anti-cheat (W-012) · dependência obrigatória da Vellcast Cloud no self-hosting (W-013) · limitar resolução no código aberto para favorecer planos pagos (W-014) · DRM próprio (W-015) · app multiplataforma no MVP (W-016) · P2P como transporte principal no Discord (W-017).

---

## 5. Pendências que dependem do responsável

| Pendência | Desbloqueia |
|---|---|
| Criar a aplicação no Discord Developer Portal (Client ID) e uma URL HTTPS pública | E0-T10 → Etapa 5 |
| Máquina Windows 11 com GPU (NVIDIA, AMD ou Intel) para testes reais | Etapas 1 (medição), 2, 4 e 6 |
| Confirmar a licença Apache-2.0 do código próprio | Marco OC |
| Controles físicos para teste (Xbox, DualShock 4, DualSense) | Validação de M-003 |
| Certificado de assinatura de código | E8-T02 |

## 6. Regras do processo

- Nenhuma tarefa é marcada como concluída sem cumprir seus critérios de aceite ([Definição de Pronto](docs/ROADMAP.md#definição-de-pronto)).
- Testes que dependem de hardware, credenciais ou acesso externo indisponível ficam como bloqueados ou pendentes de validação; resultados nunca são inventados.
- Itens Could Have não começam antes dos Must Have necessários.
- Decisões de arquitetura ficam registradas em [`docs/DECISIONS.md`](docs/DECISIONS.md).
