## O que muda

<!-- Resumo e motivo. Referencie a tarefa do roadmap (ex.: E6-T04) ou a issue. -->

## Como foi verificado

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `cargo test --workspace`
- [ ] `npm run build` em `apps/discord-activity` (se o player mudou)
- [ ] Testado em Windows real (captura, encoder, áudio, controle), se aplicável — descreva o hardware

## Checklist

- [ ] Sem segredos, tokens ou `.env` no diff
- [ ] Filas e buffers continuam limitados
- [ ] Nada roda na thread da interface que possa bloqueá-la
- [ ] Novas dependências registradas em `docs/LICENSING.md` e aceitas pelo `cargo deny`
- [ ] Documentação e roadmap atualizados
