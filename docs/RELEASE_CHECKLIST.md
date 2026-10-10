# Checklist de lançamento (E8-T07)

Use este checklist antes de publicar uma versão (tag `vX.Y.Z` na `main`; fluxo em [`CONTRIBUTING.md`](../CONTRIBUTING.md#publicar-uma-versão)). Cada linha indica **como** verificar; registre o resultado (OK / Falhou / N/A), o hardware e o link da execução da CI.

## 1. Automático (CI)

| Verificação | Onde | Cobre |
|---|---|---|
| fmt, clippy `-D warnings`, testes (Linux; desktop verificado com `cargo xwin check` e compilado no `Release`) | `CI` | Qualidade geral |
| Build do player/Activity | `CI` | M-012 (build) |
| Licenças, RustSec, duplicatas, origens | `CI` | OC-T02, M-022 |
| Segredos no histórico | `CI` | OC-T07, M-024 |
| Captura WGC real (monitor e janela) → H.264 | `Windows E2E` (manual, rodar antes da release) | M-005, M-006, M-007 (software) |
| Áudio WASAPI (sistema e processo) | `Windows E2E` / `Diagnóstico de áudio` | M-009 |
| Ponta a ponta: captura → relay → espectadores, com áudio | `Windows E2E` | M-008, M-010, M-011, M-018 |
| App renderiza as 8 telas e o teclado virtual no Windows | `Windows E2E` | M-001, M-002, M-004 |
| Pacote, instalador, instalar/abrir/desinstalar silenciosos | `Release` (antes de publicar) / `Smoke` | S-016, M-021 |
| Estabilidade de 60 min no Windows | `Windows E2E` manual com `seconds=3600` | M-025 |

## 2. Manual em Windows real (com GPU e controle)

Seguir [`TESTING_WINDOWS.md`](TESTING_WINDOWS.md) §5 (uma linha por Must). Mínimo para lançar:

| Item | Verificação | Resultado |
|---|---|---|
| Encoder de hardware | NVENC, AMD e Intel: nome em "Ao vivo" e "Video Encode" no Gerenciador de Tarefas | |
| 1080p60 | Perfil Equilibrado: 1920×1080 a ~60 fps no player (servidor próximo) | |
| Impacto no jogo | FPS médio e 1% lows com e sem transmissão (`PERFORMANCE.md` §2.2) | |
| Controle | Xbox, DualShock 4, DualSense: todas as telas só com o controle; controle não age com o app em segundo plano | |
| Teclado virtual | Digitar, apagar, maiúsculas, confirmar, cancelar | |
| Áudio | "Só o aplicativo" capta o jogo e não capta o Discord | |
| Bandeja | Fechar a janela mantém a transmissão; menu "Parar transmissão" | |
| Reconexão | Queda de rede de ~10 s: transmissor e player retomam | |
| Atualização | Release de teste mais nova que a instalada: "Verificar atualizações" → baixar → instalador abre; manifesto adulterado é recusado | |
| Instalador | Instalar, abrir pelo Menu Iniciar, desinstalar (pergunta sobre as configurações) | |
| Idiomas | pt-BR e en sem textos faltando | |

## 3. Publicação

- [ ] PR `chore(release): X.Y.Z` mergeado: versão no `Cargo.toml` (`[workspace.package]`) e no `Cargo.lock`, seção `## [X.Y.Z]` no `CHANGELOG.md` (o workflow Release recusa a tag se a versão não bater ou a seção faltar).
- [ ] Certificado de assinatura configurado (`WINDOWS_CERT_PFX_BASE64`, `WINDOWS_CERT_PASSWORD`) — sem ele o SmartScreen avisa.
- [ ] `UPDATE_SIGNING_KEY` configurado — sem ele o app recusa a atualização.
- [ ] Release publicada como **Latest** com instalador, `SHA256SUMS.txt`, `vellcast-update.json` e `.sig`; conferida com o workflow `Smoke` (campo `release_tag`).
- [ ] `docs/legal/PRIVACIDADE.md` e `TERMOS.md` revisados juridicamente e publicados no site.
- [ ] `THIRD-PARTY-NOTICES.txt` presente no pacote e no instalador.
- [ ] Deploy do relay de produção atualizado e verificado (`OPERATIONS.md` §3).
- [ ] Roadmap e `PERFORMANCE.md` atualizados com os resultados desta versão.
