# Diretrizes de UX e Design System — Vellcast

> Status: **Proposta inicial (Etapa 0)**. A implementação em Slint começa na Etapa 1.
> Nada aqui é uma interface concluída; são regras e tokens que a implementação deverá seguir.

## 1. Princípios

1. **Familiar, não copiado.** A organização (barra lateral, lista de salas, área central) é familiar para quem usa Discord, mas cores, ícones, tipografia e componentes são próprios do Vellcast. Não reutilizar logos, ícones, sons ou assets do Discord.
2. **Útil antes de bonito.** A Home mostra só o que ajuda a começar uma transmissão. Métricas técnicas ficam na seção de métricas avançadas.
3. **Controller-first.** Todo elemento interativo precisa ser alcançável por D-pad/analógico e ter foco visível.
4. **Leve.** Sem blur, sombras animadas, partículas, vídeos de fundo ou animações contínuas.
5. **Acessível.** Contraste mínimo WCAG AA (4.5:1 para texto normal, 3:1 para texto grande e componentes de UI).

## 2. Paleta (tema escuro)

Cor de destaque original: **Vellcast Teal** (`#2EC4B6`). Escolhida para não ser confundida com o roxo/azul do Discord.

| Token | Hex | Uso |
|---|---|---|
| `bg-0` | `#0E1014` | Fundo da janela / barra lateral |
| `bg-1` | `#15181E` | Lista de salas |
| `bg-2` | `#1C2028` | Área central, cartões |
| `bg-3` | `#252A34` | Campos, hover, itens selecionados |
| `text-primary` | `#ECEEF2` | Texto principal |
| `text-secondary` | `#A9B0BD` | Texto de apoio |
| `text-muted` | `#7C8494` | Legendas, placeholders (ver restrições) |
| `accent` | `#2EC4B6` | Ações primárias, estado "ao vivo" ativo |
| `accent-strong` | `#5EE0D3` | Hover/pressionado do accent, links |
| `on-accent` | `#0E1014` | Texto sobre botão `accent` |
| `danger` | `#F0625D` | Parar transmissão, erros |
| `warning` | `#F2B544` | Avisos (ex.: captura de áudio do sistema inteiro) |
| `success` | `#4CC77F` | Conectado, transmissão saudável |
| `focus` | `#FFD166` | Anel de foco (controle e teclado) |

### Contraste calculado (WCAG 2.x, luminância relativa)

Valores calculados com a fórmula WCAG a partir dos hex acima:

| Cor de primeiro plano | sobre `bg-0` | `bg-1` | `bg-2` | `bg-3` |
|---|---|---|---|---|
| `text-primary` | 16.39 | 15.30 | 14.05 | 12.39 |
| `text-secondary` | 8.73 | 8.15 | 7.49 | 6.60 |
| `text-muted` | 5.06 | 4.73 | 4.34 ⚠ | 3.83 ⚠ |
| `accent` | 8.78 | 8.20 | 7.53 | 6.64 |
| `accent-strong` | 11.86 | 11.07 | 10.17 | 8.96 |
| `danger` | 5.99 | 5.60 | 5.14 | 4.53 |
| `warning` | 10.40 | 9.71 | 8.91 | 7.86 |
| `success` | 8.87 | 8.28 | 7.60 | 6.70 |
| `focus` | 13.21 | 12.33 | 11.32 | 9.98 |

`on-accent` sobre `accent`: **8.78**.

**Restrição:** `text-muted` não atinge 4.5:1 sobre `bg-2` e `bg-3`. Nessas superfícies, usar `text-secondary` para texto de tamanho normal; `text-muted` só para texto grande (≥ 18.66 px bold / 24 px regular) ou elementos decorativos.

Estados nunca dependem apenas de cor: "ao vivo" tem ícone + rótulo; erro tem ícone + texto.

## 3. Tipografia

- Família: uma fonte sans-serif com licença OFL (candidatas: **Inter** ou **Noto Sans** para cobertura de pt-BR/en). Embutir apenas os pesos usados (400, 600) para manter o binário pequeno. Licença a registrar em `LICENSING.md`.
- Escala (px): `12` legenda · `14` corpo · `16` corpo destacado · `20` título de seção · `24` título de tela.
- Altura de linha: 1.4 para corpo, 1.2 para títulos.
- Números de métricas usam dígitos tabulares quando a fonte suportar (evita "tremor" em valores que mudam).
- Modo TV/portátil (futuro): multiplicador de escala global configurável (1.0 / 1.25 / 1.5).

## 4. Espaçamento, raios e dimensões

- Escala de espaçamento (base 4 px): `4, 8, 12, 16, 24, 32, 48`.
- Raios: `4` (campos, chips), `8` (cartões, botões), `12` (modais).
- Alvo mínimo de interação: 36 px de altura no mouse; itens de lista focáveis por controle com 44 px.
- Bordas: 1 px `bg-3` para separar superfícies; sem sombras pesadas.

## 5. Foco e estados

- **Anel de foco:** 2 px `focus` (`#FFD166`) com 2 px de afastamento do elemento. Sempre visível ao navegar por controle ou teclado; pode ser ocultado apenas quando a última interação foi com mouse.
- Exatamente **um** elemento focado por vez em toda a janela, incluindo modais.
- Ao abrir um modal, o foco vai para a ação principal; ao fechar, volta ao elemento que o abriu.
- Estados por componente: `default`, `hover`, `focused`, `pressed`, `disabled`, `selected`. `disabled` usa 40% de opacidade e não recebe foco.
- Transições: no máximo 120 ms, apenas opacidade/cor. Respeitar a preferência do sistema por movimento reduzido (sem transições).

## 6. Layout principal

```
┌────┬──────────────┬──────────────────────────────────────────┐
│    │              │                                          │
│ S  │  Salas /     │          Área central                    │
│ I  │  Transmissões│   (prévia + controles da transmissão)    │
│ D  │              │                                          │
│ E  │              │                                          │
├────┴──────────────┴──────────────────────────────────────────┤
│ Status: ● Conectado · Sala X · 1080p60   [A] Confirmar [B] Voltar │
└──────────────────────────────────────────────────────────────┘
```

- **Barra lateral (64–72 px):** ícones de navegação com rótulo em tooltip/foco: Home, Transmitir, Salas, Configurações, Conta. "Planos" e "Sobre" ficam no fim da barra.
- **Lista de salas (240 px, recolhível):** salas privadas do usuário, transmissão ativa marcada com ícone + rótulo "Ao vivo".
- **Área central:** conteúdo da tela atual. A prévia da transmissão só é renderizada quando visível (ver §8).
- **Barra inferior (32 px):** estado da conexão, sala atual, resolução/FPS ativos e **dicas do controle contextuais** (glifos Xbox ou PlayStation conforme o controle detectado; teclas quando o último input foi teclado).
- **Modo compacto (S-009):** esconde a lista de salas e mostra apenas iniciar/parar, fonte e status.
- Largura mínima de janela: 960×600. Abaixo de 1200 px a lista de salas recolhe.

## 7. Telas iniciais

| # | Tela | Conteúdo essencial | Não incluir |
|---|---|---|---|
| 01 | Home | Estado da conexão, botão "Transmitir", última sala usada, avisos relevantes (ex.: encoder indisponível) | Gráficos de CPU/GPU, listas longas |
| 02 | Transmitir | Seleção de fonte (jogos/janelas/monitores), áudio, perfil de qualidade, sala de destino, botão iniciar | Configurações avançadas fora de um expansor |
| 03 | Salas | Criar, entrar, convidar, encerrar; código de pareamento com a Activity | Descoberta pública (fora do MVP) |
| 04 | Transmissão ativa | Estado, duração, espectadores, parar, trocar fonte, ajuste rápido de qualidade, indicadores de saúde (ok/atenção) | Métricas detalhadas (ficam em "Métricas avançadas") |
| 05 | Configurações | Geral, Vídeo, Áudio, Controle (deadzone, glifos, atalhos globais — desativados por padrão), Rede, Métricas avançadas, Idioma | — |
| 06 | Conta | Login/vínculo Discord, dispositivos pareados, sair | Dados de pagamento |
| 07 | Planos | Free/Pro/Community com limites reais do serviço oficial; aviso de que self-hosting não exige plano | Pressão de venda, banners recorrentes |
| 08 | Sobre | Versão, licenças de terceiros, links para nextestudios.com, privacidade | — |

Captura sempre exige ação explícita: a tela 02 nunca inicia transmissão automaticamente, e há indicador permanente "Transmitindo" enquanto ativo (inclusive no ícone da bandeja).

## 8. Regras de desempenho da interface (seção 12 da especificação)

- A UI roda separada do motor de transmissão; captura, encoder e rede nunca executam na thread de UI.
- Atualização orientada a eventos: o motor publica mudanças de estado; a UI não faz polling intensivo. Métricas exibidas atualizam no máximo 1–2 vezes por segundo e só quando a tela de métricas está visível.
- Janela minimizada ou oculta na bandeja: nenhuma renderização, nenhuma prévia, nenhum timer de animação.
- Prévia da transmissão: desligada por padrão quando fora da tela 04; quando ligada, taxa reduzida (ex.: 10–15 fps) e resolução reduzida. Valores a validar na Etapa 2.
- Sem animações infinitas (spinners só durante operações reais, com timeout).
- Sem efeitos que exijam composição cara (blur de fundo, sombras grandes animadas, gradientes animados).
- Meta (a medir, não garantida): UI ociosa < 1% CPU e < 80 MB RAM em build Release.

## 9. Ícones e conteúdo

- Conjunto de ícones próprio ou de biblioteca com licença permissiva (ex.: Lucide — ISC), apenas os usados, embutidos como SVG.
- Texto em pt-BR e en via chaves de tradução desde a Etapa 1 (S-013). Evitar texto em imagens.
- Mensagens de erro dizem o que aconteceu e o que fazer ("O jogo foi fechado. Escolha outra fonte.").

## 10. Discord Activity (player)

A Activity segue os mesmos tokens de cor e foco, adaptados ao espaço reduzido: player ocupa a área máxima, controles sobrepostos aparecem com foco/hover e somem após inatividade. Navegação por teclado é obrigatória; por gamepad depende da validação da Gamepad API na Etapa 0.
