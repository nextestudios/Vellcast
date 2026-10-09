# Vellcast

Compartilhamento de tela otimizado para jogos: um aplicativo Windows leve, em Rust, que captura e codifica por hardware, e um player no navegador (e, em breve, uma Discord Activity) para assistir sem instalar nada. Desenvolvido pela [Next Estúdios](https://nextestudios.com).

> **Status: MVP para Windows em teste.** O app desktop captura, codifica e transmite para o `vellcast-relay`; quem recebe um link de convite assiste no navegador. A integração com o Discord foi adiada (ADR-013). Ver o [roadmap completo](ROADMAP.md) e o [guia de teste no Windows](docs/TESTING_WINDOWS.md).

## Componentes

| Componente | Caminho | Função |
|---|---|---|
| App desktop (Rust + Slint) | `apps/desktop` | Salas, convites, escolha da fonte, qualidade, áudio, tela Ao vivo, bandeja, controle e teclado virtual |
| Motor de transmissão | `crates/vellcast-core`, `crates/capture`, `crates/encoder`, `crates/audio` | Windows Graphics Capture → H.264 por hardware (Media Foundation) + áudio WASAPI → Opus |
| Transporte do transmissor | `crates/transport` | WebSocket/TLS com fila limitada e reconexão |
| Relay de mídia | `services/relay` | Salas privadas, convites, tokens, distribuição sem recodificação e player web |
| Player web / Discord Activity | `apps/discord-activity` | Player WebCodecs: `/watch` (links de convite) e modo Activity (adiado) |
| Protocolo | `crates/protocol` | Formato binário v1 (`docs/STREAMING_PROTOCOL.md`) |
| Validação da Etapa 0 | `services/stage0-server` | Servidor de mídia sintética usado para validar o player |

## Testar no Windows (sem compilar)

Baixe o artefato **Vellcast-MVP-windows-x64** em **Actions → Windows MVP**, execute `iniciar-relay.bat` e depois `vellcast-desktop.exe`. Passo a passo e checklist em [`docs/TESTING_WINDOWS.md`](docs/TESTING_WINDOWS.md).

## Desenvolvimento

Requisitos: Rust estável, Node 22+. A captura e o encoder só funcionam no Windows; no macOS/Linux a interface roda e mostra que a captura está indisponível.

```bash
# Player web
(cd apps/discord-activity && npm ci && npm run build)

# Relay servindo o player (segredo aleatório; só para desenvolvimento)
VELLCAST_RELAY_INSECURE_DEV=1 VELLCAST_STATIC_DIR=apps/discord-activity/dist \
  cargo run --release -p vellcast-relay

# App desktop (Configurações → Servidor: http://127.0.0.1:8788)
cargo run --release -p vellcast-desktop
```

Sem Windows, dá para simular um transmissor com a mídia sintética da Etapa 0 (`./scripts/generate-test-media.sh` exige ffmpeg):

```bash
curl -s -X POST localhost:8788/api/rooms -H 'content-type: application/json' -d '{"name":"teste"}'
VELLCAST_PUBLISHER_TOKEN=<publisherToken> cargo run --release -p vellcast-transport \
  --example publish_fixture -- ws://127.0.0.1:8788/ws 1080p60
```

Etapa 0 (validação do player com mídia sintética): `cargo run --release -p vellcast-stage0-server` e `http://127.0.0.1:8787`. Discord: [`docs/DISCORD_INTEGRATION.md`](docs/DISCORD_INTEGRATION.md). Docker e Dokploy: [`docs/DEPLOYMENT.md`](docs/DEPLOYMENT.md).

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

Software proprietário da Next Estúdios — todos os direitos reservados (ver [LICENSE](LICENSE) e [`docs/LICENSING.md`](docs/LICENSING.md)). Dependências de terceiros mantêm suas próprias licenças.
