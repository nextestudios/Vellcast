# Vellcast

Compartilhamento de tela otimizado para jogos: um aplicativo Windows leve, em Rust, que captura e codifica por hardware, e uma Discord Activity para assistir sem instalar nada. Desenvolvido pela [Next Estúdios](https://nextestudios.com).

> **Status: ETAPA 0 — validação da Discord Activity.** O aplicativo Windows ainda não existe. Esta etapa comprova, com mídia sintética, que H.264 + Opus via WebSocket e WebCodecs funciona dentro de uma Activity antes de investir no motor de captura. Ver [`docs/ROADMAP.md`](docs/ROADMAP.md).

## O que existe hoje

| Componente | Caminho | Estado |
|---|---|---|
| Protocolo binário v1 | `crates/protocol` | Implementado e testado |
| Servidor de validação (Axum) | `services/stage0-server` | Implementado; serve a Activity, distribui mídia pré-codificada para N espectadores |
| Discord Activity (React + WebCodecs) | `apps/discord-activity` | Implementado; validado fora do Discord, validação no Discord pendente |
| Desktop, captura, encoder, áudio, relay | `apps/desktop`, `crates/*`, `services/relay` | Planejado (ETAPAS 1–5) |

## Rodando a ETAPA 0 localmente

Requisitos: Rust estável, Node 22+, ffmpeg com libx264 e libopus (só para gerar a mídia de teste).

```bash
./scripts/generate-test-media.sh          # gera tests/fixtures/media/ (não versionado)
(cd apps/discord-activity && npm ci && npm run build)
cargo run --release -p vellcast-stage0-server
# abra http://127.0.0.1:8787 e escolha uma transmissão
```

Desenvolvimento da Activity com hot reload: `cd apps/discord-activity && npm run dev` (porta 5173; `/ws` e `/api` são encaminhados ao servidor na 8787).

Teste de carga: `node tests/benchmarks/stage0-viewers.mjs ws://127.0.0.1:8787 1080p60 20 30`.

Para testar dentro do Discord, siga [`docs/DISCORD_INTEGRATION.md`](docs/DISCORD_INTEGRATION.md). Docker e Dokploy: [`docs/DEPLOYMENT.md`](docs/DEPLOYMENT.md).

## Verificações

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test
(cd apps/discord-activity && npm run build)
```

A CI roda o mesmo em Linux e Windows (`.github/workflows/ci.yml`). Testes de captura, GPU, áudio e FPS exigem uma máquina Windows real.

## Documentação

| Tema | Documento |
|---|---|
| Produto e requisitos | [PRD](docs/PRD.md), [MOSCOW](docs/MOSCOW.md), [ROADMAP](docs/ROADMAP.md), [RISKS](docs/RISKS.md) |
| Arquitetura | [ARCHITECTURE](docs/ARCHITECTURE.md), [TECH_STACK](docs/TECH_STACK.md), [DECISIONS](docs/DECISIONS.md), [STREAMING_PROTOCOL](docs/STREAMING_PROTOCOL.md) |
| Discord | [DISCORD_INTEGRATION](docs/DISCORD_INTEGRATION.md) |
| Qualidade | [PERFORMANCE](docs/PERFORMANCE.md), [SECURITY](docs/SECURITY.md) |
| Experiência | [UX_GUIDELINES](docs/UX_GUIDELINES.md), [CONTROLLER_SUPPORT](docs/CONTROLLER_SUPPORT.md) |
| Negócio e licenças | [MONETIZATION](docs/MONETIZATION.md), [OPEN_SOURCE_STRATEGY](docs/OPEN_SOURCE_STRATEGY.md), [LICENSING](docs/LICENSING.md) |
| Operação | [DEPLOYMENT](docs/DEPLOYMENT.md) |

## Licença

Código próprio sob [Apache-2.0](LICENSE) (**proposta, pendente de confirmação** — ver [`docs/LICENSING.md`](docs/LICENSING.md)). Dependências de terceiros mantêm suas próprias licenças.
