# Suporte a Controles — Vellcast

> Status: **Implementado na Etapa 1 (software); nenhum controle físico testado ainda.** A lógica (mapeamento, deadzone, repetição, roteamento) tem testes unitários e o fluxo completo foi exercitado com ações simuladas pelo mesmo caminho do gamepad real (`VELLCAST_DEV_ACTIONS`, ver `apps/desktop/README.md`). Não prometer suporte universal sem testes em hardware.

## 1. Arquitetura (implementada)

Crate `crates/controller/` (`vellcast-controller`):

| Arquivo | Responsabilidade | Testes |
|---|---|---|
| `mapping.rs` | Botão posicional (South/East/West/North…) → ação. Os nomes posicionais do GilRs dão o mesmo resultado para Xbox e PlayStation | 4 |
| `stick.rs` | Analógico → direção discreta, com deadzone radial configurável e histerese (ativa em 50% do curso útil, solta em 35%) | 4 |
| `repeat.rs` | Repetição: dispara ao pressionar, espera o atraso, repete no intervalo; sem rajadas após pausas longas | 6 |
| `worker.rs` | Thread `vellcast-gamepad` com GilRs 0.11, bloqueada em `next_event_blocking` (sem polling ocioso); emite eventos só com a janela ativa | — (exige hardware) |
| `lib.rs` | `Action`, `ControllerEvent`, `InputConfig` com limites seguros | 2 |

- **Camada:** GilRs 0.11.2 (no Windows usa Windows.Gaming.Input/XInput). XInput direto só se testes em hardware mostrarem lacunas (ADR-016).
- **Saída:** eventos semânticos (`Action::Confirm`, `Action::NextSection`…) e `Connected`/`Disconnected`. A UI não conhece o hardware.
- **No app** (`apps/desktop/src/input.rs`, 6 testes): fora de modais, Baixo/Cima viram Tab/Shift+Tab, Esquerda/Direita viram setas e A vira Enter na cadeia de foco do Slint. Com modal aberto, as ações vão direto ao modal (ADR-015).

## 2. Mapeamento inicial

| Ação | Xbox | PlayStation |
|---|---|---|
| Confirmar | A | ✕ (Cruz) |
| Voltar | B | ○ (Círculo) |
| Ação secundária | X | □ (Quadrado) |
| Opções | Y | △ (Triângulo) |
| Seção anterior | LB | L1 |
| Próxima seção | RB | R1 |
| Aba anterior | LT | L2 |
| Próxima aba | RT | R2 |
| Navegação | Analógico esquerdo / D-pad | Analógico esquerdo / D-pad |
| Scroll | Analógico direito | Analógico direito |
| Menu | Start (Menu) | Options |
| Ações rápidas | Select (View) | Create |

Nota: em alguns layouts japoneses Confirmar/Voltar são invertidos no PlayStation. Fica como opção de remapeamento futura, não padrão.

## 3. Regras

- Navegação 100% acessível por controle para todas as funções essenciais (M-003).
- Foco visual sempre visível (ver `UX_GUIDELINES.md` §5).
- Modais e o teclado virtual são navegáveis por controle e prendem o foco enquanto abertos.
- **Repetição de input controlada:** padrão de 400 ms de atraso e 110 ms de intervalo, ajustáveis em Configurações (150–1000 ms e 40–500 ms). Só direções e rolagem repetem; Confirmar/Voltar nunca.
- **Deadzone configurável:** padrão 0,20 radial, ajustável de 0,05 a 0,50 em Configurações. Os gatilhos são lidos como botões digitais pelo GilRs.
- **Navegação em ordem** (cadeia de foco), não espacial: Baixo avança e Cima volta. Esquerda/Direita ajustam o controle focado. A navegação espacial fica para avaliar em testes com usuários (ADR-015).
- **Funções atuais dos botões no app:** A confirma; B volta pelo histórico de telas (até 32); LB/RB trocam de seção e põem o foco no item da barra lateral; LT/RT trocam de aba (hoje só a tela Planos tem abas); analógico direito rola a área central; Start abre o menu rápido (Início, Configurações, Minimizar para a bandeja, Sair). X, Y e Select não têm função fora do teclado virtual nesta etapa.
- Remapeamento: estrutura de dados preparada; UI de remapeamento é futura.
- Detecção de conexão/desconexão (S-010): a lista de controles na Home e na barra de status se atualiza. Ao desconectar, o estado pressionado é descartado; o foco atual permanece onde estava, então a navegação continua de onde parou quando o controle volta.
- Mouse, teclado e controle funcionam simultaneamente; glifos da barra inferior seguem o último dispositivo usado.
- Teclado: setas navegam, Enter confirma, Esc volta, Tab/Shift+Tab ciclam.

## 4. Comportamento em segundo plano

- Quando a janela do Vellcast **não está em foco**, o módulo **não reage** a botões comuns do gamepad. O jogo recebe os comandos normalmente; o Vellcast não consome nem bloqueia input (não usa hooks globais de input nem injeção).
- **Atalhos globais** (ex.: combinação para parar a transmissão) são **opcionais e desativados por padrão**. Quando ativados, devem usar combinações improváveis em jogos (ex.: segurar Select + Start por 1 s) e aparecer claramente nas configurações.
- Atalhos globais de teclado usam a API de hotkeys do Windows (`RegisterHotKey`), também desativados por padrão.
- Avaliar se o botão Guide/PS pode ser lido sem conflitar com Xbox Game Bar/Steam — não usar por padrão.

## 5. Teclado virtual (M-004)

Original, inspirado na experiência dos consoles, sem copiar assets.

- Layout em grade: letras (QWERTY), números, símbolos (página alternável), espaço, apagar, maiúsculas/minúsculas, confirmar, cancelar.
- Navegação por D-pad e analógico; tecla selecionada com o mesmo anel de foco do sistema.
- **Implementado** (`apps/desktop/ui/keyboard.slint`): três páginas (letras, números e símbolos, acentos do pt-BR), Shift de um uso, espaço, apagar, cancelar e OK, limite de caracteres e contador.
- Atalhos implementados: X/□ apaga, Y/△ insere espaço, Start/Options confirma, B/○ cancela. Shift e troca de página são teclas da grade (LT/RT ainda não estão ligados ao teclado).
- Abre quando o campo de texto é ativado **pelo controle** (A no campo). Com mouse ou teclado, o campo edita direto. Dentro do teclado virtual, o campo de prévia é um campo real: o teclado físico continua funcionando (Enter confirma, Esc cancela). Ao fechar, o foco volta ao campo que o abriu.
- Campos sensíveis (ex.: tokens) não exibem histórico nem sugestões.

## 6. Matriz de compatibilidade

Todos os itens abaixo estão **Não testado**. Status só muda após teste real em Windows 11, registrando hardware, conexão (USB/Bluetooth), versão do driver e resultado.

| Controle | Conexão | Detecção | Navegação | Desconexão/reconexão | Segundo plano sem interferência | Status |
|---|---|---|---|---|---|---|
| Xbox One | USB | — | — | — | — | Não testado |
| Xbox One | Bluetooth | — | — | — | — | Não testado |
| Xbox Series X\|S | USB | — | — | — | — | Não testado |
| Xbox Series X\|S | Bluetooth | — | — | — | — | Não testado |
| DualShock 4 | USB | — | — | — | — | Não testado |
| DualShock 4 | Bluetooth | — | — | — | — | Não testado |
| DualSense | USB | — | — | — | — | Não testado |
| DualSense | Bluetooth | — | — | — | — | Não testado |
| Genérico XInput | USB | — | — | — | — | Não testado |
| Genérico DirectInput | USB | — | — | — | — | Não testado |
| Steam Deck (Windows) | Integrado | — | — | — | — | Não testado |
| ROG Ally | Integrado | — | — | — | — | Não testado |
| Legion Go | Integrado | — | — | — | — | Não testado |

## 7. Discord Activity

O suporte à Gamepad API dentro da Activity (iframe do Discord) é verificado na Etapa 0. Se não estiver disponível ou exigir permissão do iframe, o player usa navegação por teclado e mouse como fallback. Resultado registrado em `DISCORD_INTEGRATION.md`.

## 8. Testes necessários (Etapa 1)

- Unitários (**feitos**: 16 no `vellcast-controller` e 6 de roteamento no app): mapeamento botão → ação, deadzone e histerese, repetição, roteamento com e sem modal, histórico.
- Fluxo simulado (**feito** no macOS): RB até Configurações → navegação por foco → teclado virtual digita e confirma → foco volta ao campo → ajuste da deadzone → valores gravados em `settings.json`; troca de idioma ao vivo; Voltar até a Home.
- Manuais em Windows: cada linha da matriz, conexão/desconexão durante uso, controle + mouse simultâneos, jogo em primeiro plano com Vellcast em segundo plano (confirmar que nenhuma ação ocorre).
