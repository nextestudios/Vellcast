# Estratégia Open Core — Vellcast

> Status: **Proposta**. A publicação de qualquer componente depende da validação de licenças (`LICENSING.md`).

## 1. Divisão

| Camada | Conteúdo | Licença proposta |
|---|---|---|
| **Vellcast Core** (aberto) | `apps/desktop`, `apps/discord-activity`, `crates/*` (capture, encoder, audio, controller, transport, protocol, metrics, auth, vellcast-core), `services/relay` | Apache-2.0 (pendente confirmação) — exceto onde Slint impuser condições (ver `LICENSING.md`) |
| **Vellcast Cloud** (comercial) | `services/cloud-api` (contas, billing, entitlements, cotas), operação dos servidores oficiais, painel administrativo, hospedagem gerenciada | Proprietário; pode viver em repositório privado separado |

Regra de fronteira: o Core deve funcionar de ponta a ponta sozinho (desktop → relay self-hosted → Activity/player). O Cloud apenas adiciona contas, cotas e operação gerenciada.

## 2. Compromissos públicos

1. Self-hosting do relay básico sem assinatura e sem chamada obrigatória ao Vellcast Cloud (W-013).
2. Nenhum bloqueio artificial de resolução/FPS no código aberto (W-014). 1080p60 é suportado no Core.
3. Sem telemetria de conteúdo. Métricas técnicas anônimas, se existirem, serão opt-in e documentadas.
4. Protocolo de streaming documentado (`STREAMING_PROTOCOL.md`) para permitir clientes e servidores alternativos.

## 3. Como o comercial se conecta ao aberto

- O relay expõe um ponto de extensão de **autorização/limites** (trait em Rust). A implementação aberta lê limites de configuração; a implementação Cloud consulta entitlements.
- O desktop conhece apenas "servidor + token". Não há código de pagamento no desktop; a tela "Planos" abre o site/portal.
- A Activity oficial aponta para o serviço oficial; operadores self-hosted podem registrar sua própria Activity no Discord Developer Portal apontando para seu relay.

## 4. Governança e contribuição

- `CONTRIBUTING.md` com fluxo de PR, padrões de código (rustfmt, Clippy, testes) e requisitos de CI.
- Contribuições sob a mesma licença do projeto (inbound = outbound, cláusula 5 da Apache-2.0). Avaliar DCO (`Signed-off-by`) em vez de CLA, para reduzir atrito.
- Marca "Vellcast", nome e logo **não** são licenciados pela Apache-2.0 (cláusula 6). Forks devem usar outro nome. Política de marca a publicar antes do lançamento.
- Segurança: canal privado de reporte de vulnerabilidades (`SECURITY.md`).

## 5. Quando abrir

| Momento | Ação |
|---|---|
| Agora (Etapa 0) | Repositório pode ser público com docs, protocolo, servidor de validação e Activity de teste — **após** confirmar a licença Apache-2.0 |
| Antes da Etapa 1 | Decidir a licença do Slint (bloqueia a distribuição do desktop) |
| Antes da Etapa 8 | Inventário de licenças completo, NOTICE, política de marca, termos e privacidade |

## 6. Riscos

- **Slint sob GPLv3** tornaria o desktop distribuído sob GPLv3, o que conflita com a ideia de desktop Apache-2.0 (ver `LICENSING.md`).
- Codecs: H.264 e AAC têm questões de patentes. Usar encoders de hardware do sistema (Media Foundation/drivers) e Opus (livre de royalties) reduz a exposição; não distribuir libx264/FFmpeg com GPL.
- Concorrentes podem hospedar o Core comercialmente — aceito pelo modelo; diferencial do Cloud é operação, qualidade e integração oficial.
