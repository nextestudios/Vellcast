# Suporte a Controles — Vellcast

> Status: **Planejado (Etapa 1)**. Nenhum controle foi testado ainda. Não prometer suporte universal sem testes.

## 1. Arquitetura

Módulo independente `crates/controller/`:

- **Camada principal candidata:** [GilRs](https://gitlab.com/gilrs-project/gilrs) (Windows: XInput + Windows.Gaming.Input). Avaliar na Etapa 1: manutenção, licença (MIT/Apache-2.0), suporte a DualSense/DualShock via WGI, comportamento em segundo plano.
- **Fallback nativo:** XInput / Windows.Gaming.Input diretamente quando GilRs não atender (ex.: detecção de foco, controles de portáteis).
- Saída do módulo: eventos semânticos de navegação (`Confirm`, `Back`, `Move(dir)`, `Scroll(dy)`, `NextSection`…), não botões crus. A UI Slint consome esses eventos; não conhece o hardware.
- Thread própria com espera por eventos; sem polling contínuo em alta frequência quando a janela não está focada.

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
- **Repetição de input controlada:** atraso inicial ~400 ms, depois repetição ~80 ms ao segurar direção (valores a ajustar em teste).
- **Deadzone configurável** (padrão proposto: 0.25 radial nos analógicos; gatilhos como digitais com limiar 0.5).
- Navegação espacial (para cima/baixo/esquerda/direita) por geometria dos elementos, com ordem explícita quando a geometria for ambígua.
- Remapeamento: estrutura de dados preparada; UI de remapeamento é futura.
- Detecção de conexão/desconexão; ao reconectar, a navegação retoma o último foco (S-010).
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
- Atalhos: X/□ apaga, Y/△ espaço, LT/L2 alterna maiúsculas, RT/R2 alterna símbolos, Start/Options confirma, B/○ cancela (valores iniciais, a validar com usuários).
- Aparece automaticamente quando um campo de texto recebe foco **por controle**; nunca impede o teclado físico.
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

- Unitários: mapeamento botão → ação, deadzone, repetição, navegação espacial (sem hardware).
- Manuais em Windows: cada linha da matriz, conexão/desconexão durante uso, controle + mouse simultâneos, jogo em primeiro plano com Vellcast em segundo plano (confirmar que nenhuma ação ocorre).
