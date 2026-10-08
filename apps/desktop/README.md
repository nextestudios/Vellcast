# apps/desktop — Vellcast para Windows

Aplicativo desktop em Rust + [Slint](https://slint.dev) 1.18 (licença Royalty-free 2.0, ADR-006). Sem Electron e sem webview.

**Estado (Etapa 1, primeiro incremento):** a interface, a navegação por controle, o teclado virtual, a bandeja e as configurações funcionam. Captura, salas, transmissão e conta são etapas futuras e aparecem como avisos, nunca como dados simulados.

## Rodar

```bash
cargo run -p vellcast-desktop                # debug
cargo run --release -p vellcast-desktop      # release
```

- Configurações: `settings.json` no diretório do sistema (Windows: `%APPDATA%\Next Estudios\Vellcast\config\`; macOS: `~/Library/Application Support/com.Next-Estudios.Vellcast/`). Um arquivo corrompido é renomeado para `.corrupt` e o app volta aos padrões.
- Logs: `RUST_LOG=debug`.
- Renderer: software por padrão (ADR-014). Para comparar com OpenGL: `cargo run --release -p vellcast-desktop --features gpu-renderer` e `SLINT_BACKEND=winit-femtovg`.

## Estrutura

| Caminho | Conteúdo |
|---|---|
| `src/main.rs` | Inicialização, ligação UI ↔ configurações ↔ controle ↔ bandeja |
| `src/input.rs` | Roteamento das ações do controle (testado) |
| `src/settings.rs` | Configurações persistidas (testado) |
| `src/tray.rs` | Ícone e menu da bandeja |
| `src/dev.rs` | Snapshot e ações simuladas (feature `dev-snapshot`, só desenvolvimento) |
| `ui/theme.slint` | Tokens de `docs/UX_GUIDELINES.md` |
| `ui/widgets.slint` | Componentes navegáveis por foco |
| `ui/keyboard.slint` | Teclado virtual |
| `ui/pages.slint` | Telas 01–08 |
| `ui/app.slint` | Janela, barra lateral, barra de status, modais |
| `translations/pt-BR/` | Catálogo pt-BR embutido no binário |

## Traduções (S-013)

As strings da UI ficam em `@tr("...")`, com o inglês como idioma-fonte. O pt-BR fica em `translations/pt-BR/LC_MESSAGES/vellcast-desktop.po` e é embutido no build. Ao adicionar uma string, inclua a tradução no `.po`: o teste `every_ui_string_has_pt_br_translation` falha se faltar alguma.

## Verificação sem controle físico (feature `dev-snapshot`)

```bash
cargo build -p vellcast-desktop --features dev-snapshot
VELLCAST_DEV_SNAPSHOT=/tmp/tela.bmp VELLCAST_DEV_PAGE=4 ./target/debug/vellcast-desktop
VELLCAST_DEV_SNAPSHOT=/tmp/fluxo.bmp \
  VELLCAST_DEV_ACTIONS=rb,rb,rb,rb,down,down,down,down,down,confirm,confirm,right,confirm,menu,down,right \
  ./target/debug/vellcast-desktop
```

`VELLCAST_DEV_ACTIONS` passa pelo mesmo `handle_action` usado pelo gamepad real; só a leitura do hardware (GilRs) fica de fora. Também aceita `VELLCAST_DEV_MODAL=menu|keyboard` e `VELLCAST_DEV_TABS=<n>`. A captura grava a janela em BMP e encerra o app.

## Medição de CPU e memória

Metodologia usada nos números do ADR-014: build Release; app iniciado com um `HOME` temporário (configurações limpas); 5 s de espera; amostragem por 20–30 s. CPU = variação do tempo de CPU do processo (`ps -o cputime`) dividida pelo tempo de parede. Memória = `phys_footprint` (`footprint -p <pid>`), que é o que o Monitor de Atividade mostra. O RSS do macOS inclui páginas compartilhadas de frameworks do sistema e não serve como comparação.

| Cenário (macOS, Apple M1, 2026-10-08) | CPU média | Memória (`phys_footprint`) |
|---|---|---|
| Janela visível, ociosa, renderer de software | 0,05–0,27% de 1 núcleo | 32–33 MB |
| Oculto na bandeja (início minimizado) | 0,03–0,10% de 1 núcleo | 20 MB |
| Janela visível, femtovg (OpenGL) | 0,05% de 1 núcleo | 41 MB |

Máquina com outras cargas durante a medição (load average 8–18). **Windows: pendente**, medir com o Gerenciador de Tarefas/`perfmon` (conjunto de trabalho privado) e registrar em `docs/PERFORMANCE.md`.

Binário Release no macOS: ~15 MB sem `strip`.
