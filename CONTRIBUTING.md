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

## Commits

Mensagens curtas no imperativo, explicando o porquê quando não for óbvio. Documentação em português brasileiro; código e identificadores em inglês.
