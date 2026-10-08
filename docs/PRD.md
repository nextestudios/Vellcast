# Vellcast — Documento de Requisitos do Produto (PRD)

| Campo | Valor |
|---|---|
| Produto | Vellcast |
| Empresa | Next Estúdios (https://nextestudios.com) |
| Plataforma inicial | Windows 11 (transmissor) · Discord Activity (espectador) |
| Modelo | Open Core + Freemium |
| Status | Planejamento · ETAPA 0 em andamento |

Documentos relacionados: [ARCHITECTURE](ARCHITECTURE.md) · [MOSCOW](MOSCOW.md) · [ROADMAP](ROADMAP.md) · [PERFORMANCE](PERFORMANCE.md) · [SECURITY](SECURITY.md) · [MONETIZATION](MONETIZATION.md).

## 1. Problema

Quem quer mostrar um jogo para amigos no Discord hoje esbarra em três coisas:

1. **Qualidade presa atrás de assinatura.** O compartilhamento nativo limita resolução e FPS no plano gratuito.
2. **Custo no desempenho do jogo.** Os transmissores costumam rodar em cima de navegadores ou runtimes pesados e consomem CPU, RAM e GPU que o jogo precisaria.
3. **Experiência ruim com controle.** Quem joga com gamepad, inclusive em PCs portáteis, tem que largar o controle para mexer no transmissor.

## 2. Visão

O Vellcast é um transmissor de tela **especializado em jogos**. Ele é extremamente leve, usa captura e codificação nativas por hardware e permite que espectadores assistam **dentro do Discord**, via Activity, sem instalar nada. O diferencial não é ter muitas funções: é ter a melhor transmissão pelo menor custo de recursos.

O Vellcast não substitui o Discord. A voz continua no próprio Discord.

## 3. Personas

| Persona | Necessidade | Critério de sucesso |
|---|---|---|
| **Transmissor gamer** | Mostrar o jogo para amigos em 1080p60 sem perder FPS | Inicia em poucos passos e nota pouco impacto no jogo |
| **Jogador de controle / PC portátil** | Operar tudo pelo gamepad | Nunca precisa do mouse no fluxo principal |
| **Espectador** | Assistir sem instalar nada | Entra na Activity e vê a transmissão autorizada |
| **Comunidade / self-hoster** | Hospedar o próprio relay | Sobe o servidor com Docker, sem depender da Vellcast Cloud |

## 4. Fluxos principais

### 4.1 Transmissor (Windows)
1. Abrir o Vellcast.
2. Entrar ou conectar ao serviço (oficial ou self-hosted).
3. Selecionar o jogo ou o monitor (escolha explícita).
4. Selecionar ou criar uma sala privada.
5. Ajustar a qualidade (opcional; o padrão é um perfil equilibrado).
6. Iniciar a transmissão.
7. Minimizar para a bandeja e continuar jogando.

### 4.2 Espectador (Discord)
1. Abrir o Discord e iniciar a Activity Vellcast.
2. Ver as transmissões autorizadas da sala ou do canal.
3. Selecionar uma transmissão.
4. Assistir com áudio, volume e tela cheia quando suportada.

## 5. Requisitos funcionais (resumo)

A classificação completa, com critérios de aceite, está em [MOSCOW.md](MOSCOW.md).

| Área | Requisitos MUST |
|---|---|
| Desktop | App nativo Rust/Slint (M-001), UI amigável (M-002), controle completo (M-003), teclado virtual (M-004), bandeja (M-017) |
| Captura | Janela (M-005), monitor (M-006), áudio por processo ou sistema (M-009) |
| Codificação | Encoder por hardware com detecção (M-007), 1080p60 gratuito (M-008) |
| Distribuição | Relay Rust (M-010), múltiplos espectadores sem recodificação (M-011), reconexão (M-019), segurança (M-020) |
| Discord | Activity com player WebCodecs (M-012) |
| Acesso | Salas privadas (M-013), autenticação (M-014), pareamento desktop↔Activity (M-015) |
| Operação | Controles de transmissão (M-016), limites de recursos (M-018), self-hosting (M-021), documentação (M-022), métricas (M-023), privacidade (M-024), testes de estabilidade (M-025) |

## 6. Requisitos não funcionais

| Requisito | Meta de engenharia (não é garantia; ver PERFORMANCE.md) |
|---|---|
| App ocioso | CPU < 1%, RAM < 80 MB, nenhuma renderização quando minimizado |
| Transmissão | 1920×1080 a 60 FPS, H.264 por hardware, 8 Mbps como referência de teste |
| Impacto no jogo | Perda < 5% de FPS médio em hardware de referência; medir também 1% lows e frametime |
| Latência | < 500 ms ponta a ponta em condições favoráveis |
| Estabilidade | Sessões ≥ 60 min sem crescimento de memória |
| UI | Nenhuma operação de captura ou rede na thread da interface |
| Acessibilidade | 100% navegável por gamepad, foco sempre visível |

## 7. Princípios inegociáveis

- Não usar navegador completo nem Electron no transmissor (W-001, W-002).
- Captura somente com ação explícita do usuário; nada de captura escondida, injeção em processos ou hooks em jogos (W-007, W-011, W-012).
- O plano Free inclui 1080p60 com áudio. A decisão é definitiva.
- O self-hosting do relay não depende de assinatura (W-013, W-014).
- Nenhuma meta de desempenho é declarada atingida sem medição reproduzível.

## 8. Planos

| Plano | Escopo |
|---|---|
| **Free** | Até 1080p60, áudio, encoder por hardware, controle, salas privadas, servidores oficiais dentro da cota gratuita, sem marca d'água |
| **Pro** | Free + 1440p60/4K60 quando suportado, mais espectadores, cotas ampliadas, personalização |
| **Community** | Pro + gestão de comunidades, salas permanentes, cotas comunitárias, hospedagem gerenciada |

Preços e cotas ainda não estão definidos. Dependem de testes de capacidade e custo de banda (ver [MONETIZATION.md](MONETIZATION.md)).

## 9. Fora do escopo do MVP

Voz própria, substituição do Discord, WebRTC na Activity, recodificação obrigatória no servidor, gravação automática, apps Linux/macOS, P2P como transporte principal e DRM próprio. A lista completa (W-001…W-017) está em [MOSCOW.md](MOSCOW.md).

## 10. Métricas de sucesso do MVP

1. Um usuário inicia a transmissão no Windows e outro assiste dentro do Discord (critério da ETAPA 5).
2. O fluxo completo do transmissor funciona somente com gamepad.
3. 1080p60 é sustentado por 60 min em hardware de referência, com o relatório publicado em PERFORMANCE.md.
4. O relay atende N espectadores sem recodificar. N será definido em teste de carga.

## 11. Premissas e dependências externas

- Uma aplicação Discord registrada, com Activities habilitadas e URL Mappings configurados.
- WebCodecs (`VideoDecoder` e `AudioDecoder`) disponíveis no webview do Discord. **A ETAPA 0 existe para validar isso.**
- Máquina Windows real para testes de captura, GPU, áudio e FPS. O ambiente de desenvolvimento atual é macOS.
- As políticas de Premium Apps do Discord para desenvolvedores no Brasil ainda precisam ser verificadas antes de qualquer cobrança.
