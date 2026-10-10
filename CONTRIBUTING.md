# Contribuindo com o Vellcast

## Antes de começar

- Leia o [Código de Conduta](CODE_OF_CONDUCT.md). Vulnerabilidades: [SECURITY.md](SECURITY.md) (nunca em issues públicas).

- Leia [`docs/ROADMAP.md`](docs/ROADMAP.md) e [`docs/MOSCOW.md`](docs/MOSCOW.md). Itens Could Have não entram antes dos Must Have necessários.
- Decisões de arquitetura ficam em [`docs/DECISIONS.md`](docs/DECISIONS.md). Mudar uma decisão aceita exige um novo ADR com a justificativa técnica.

## Regras de código

- Rust estável, `cargo fmt` e `cargo clippy -- -D warnings` sem avisos.
- Sem `unwrap`/`expect` em caminhos críticos; erros tratados e registrados com `tracing`.
- `unsafe` é proibido no workspace por padrão (`unsafe_code = "forbid"`). Crates que precisarem de FFI do Windows devem liberar localmente e documentar cada bloco `unsafe` com a invariante que o torna seguro.
- Filas sempre limitadas. Nenhum buffer cresce sem limite.
- Nenhuma operação de captura ou rede na thread da interface.
- Novas dependências: verificar manutenção, licença, tamanho, suporte a Windows e impacto no desempenho, e registrar em `docs/TECH_STACK.md` e `docs/LICENSING.md`. O job `cargo deny` da CI recusa licenças fora de `deny.toml`.

## Testes e resultados

- Toda mudança em protocolo, salas, filas ou autorização vem com testes.
- Medições de desempenho só em build Release, com hardware, sistema, rede e metodologia registrados em `docs/PERFORMANCE.md`.
- Nunca marque uma tarefa como Done sem cumprir os critérios de aceite. Testes que dependem de hardware, credenciais ou acesso externo indisponível ficam como **Blocked** ou **Pending Validation**.

## Segurança e privacidade

- Nunca versionar tokens, segredos ou `.env` (use `.env.example`).
- Nenhuma captura sem ação explícita do usuário; nenhuma injeção em processos de jogos.
- Vulnerabilidades: reporte pelo GitHub Private Vulnerability Reporting (ver [SECURITY.md](SECURITY.md)), nunca em issues públicas.

## Fluxo de branches (Gitflow)

Seguimos o [Gitflow](https://www.atlassian.com/git/tutorials/comparing-workflows/gitflow-workflow), com uma regra temporária:

> **Antes da 1.0 (fase atual):** não existe `develop`. PRs de `feature/*`, `fix/*` e `hotfix/*` vão direto para `main`; as versões são marcadas com tag na `main`. O modelo completo abaixo (com `develop` e `release/*`) começa na primeira versão estável.

- `main` guarda só o que pode ser publicado (cada versão tem a tag `vX.Y.Z`). `develop` integra o que vai para a próxima versão.
- **Nova funcionalidade/correção comum:** `feature/<issue>-<nome>` a partir de `develop` → pull request para `develop`.
- **Preparar versão:** `release/X.Y.Z` a partir de `develop` (só correções, changelog e docs) → pull request para `main` → tag no merge (publica a release) → merge de `main` de volta em `develop`.
- **Correção urgente:** `hotfix/X.Y.Z` a partir de `main` → pull request para `main` → tag → merge de volta em `develop`.
- **Pull requests:** título no formato [Conventional Commits](https://www.conventionalcommits.org/pt-br/) (`feat: …`, `fix: …`, `docs: …`, `ci: …`, `chore(release): X.Y.Z`), merge por squash. Nada vai direto para a `main` sem PR.

### Onde a CI roda

| Workflow | Quando | O que faz |
|---|---|---|
| **CI** | PR para `main`/`develop` (exceto só docs) | fmt, clippy, testes (Linux e Windows), build do player, licenças, varredura de segredos |
| **Release** | tag `vX.Y.Z` na `main` | testes completos, pacote, instalador, smoke (instalar/abrir/desinstalar), manifesto de atualização assinado e release publicada |
| **CodeQL** | push na `main` e semanal | análise de segurança (ativa quando o repositório for público) |
| **Smoke** | manual | gera pacote e instalador de um commit, ou testa o instalador de uma release publicada |
| **Windows E2E** | manual | captura → encoder → relay → espectadores, bitrate adaptativo, modo competitivo, app e bandeja |
| **Diagnóstico de áudio** | manual | captura WASAPI sob o depurador com page heap |

PRs não geram pacotes. Rode o **Windows E2E** antes de uma release que mexa em captura, encoder, áudio, transporte ou relay.

### Publicar uma versão

Só quando o mantenedor pedir; PRs são mergeados conforme ficam prontos, sem gerar release.

1. PR `chore(release): X.Y.Z`: mova o conteúdo de `## [Unreleased]` do [`CHANGELOG.md`](CHANGELOG.md) para `## [X.Y.Z]` (com **Limitações conhecidas**), atualize `version` no `Cargo.toml` e rode `cargo update --workspace --offline` para o `Cargo.lock`.
2. Depois do merge: `git tag vX.Y.Z origin/main && git push origin vX.Y.Z`.
3. O workflow **Release** confere a tag (precisa estar na `main` e bater com o `Cargo.toml`), roda os testes, gera e testa o instalador e publica a release como **Latest** com as notas do changelog. Checklist completo em [`docs/RELEASE_CHECKLIST.md`](docs/RELEASE_CHECKLIST.md).

## Commits

Mensagens curtas no imperativo, explicando o porquê quando não for óbvio. Documentação em português brasileiro; código e identificadores em inglês.
