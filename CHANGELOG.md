# Histórico de alterações

Notas em português do Brasil. Antes de publicar uma versão, adicione a seção `## [VERSÃO]` (no PR `chore(release): VERSÃO`, junto com a versão do `Cargo.toml`). O workflow Release usa a seção da tag como notas da release e falha se ela faltar. O formato segue o [Keep a Changelog](https://keepachangelog.com/pt-BR/1.1.0/).

## [Unreleased]
### Mudanças
- **Fluxo de trabalho no GitHub no padrão Gitflow** (o mesmo do ControlFS): CI só em pull requests para `main`/`develop`; release publicada por tag `vX.Y.Z` na `main`, com notas deste arquivo, testes completos e smoke do instalador antes de publicar; workflows Smoke, Windows E2E e Diagnóstico de áudio sob demanda; Dependabot mensal agrupado; CodeQL pronto para quando o repositório for público.

## [0.1.2]
### Correções
- **"Não foi possível falar com o servidor" ao criar salas:** o app passa a validar certificados pelo próprio Windows, como o navegador. Antivírus que inspecionam HTTPS (Kaspersky, Avast, ESET, Bitdefender) instalam um certificado próprio que o app não reconhecia. A transmissão também passa a usar os certificados do sistema.
- Mensagens de erro e o log agora mostram o motivo técnico da falha.

### Limitações conhecidas
- Log do app: `%LOCALAPPDATA%\Next Estudios\Vellcast\data\logs\vellcast-desktop.log`.
- Executável sem assinatura de código: o SmartScreen pode avisar na primeira execução.

## [0.1.1]
### Novidades
Primeira versão de teste do Vellcast: transmita seu jogo do Windows e assista no navegador por link de convite.
- Captura nativa (Windows Graphics Capture) e H.264 por hardware, com fallback por software.
- Áudio do jogo por processo (WASAPI + Opus), sem capturar a voz do Discord.
- Bitrate adaptativo e modo competitivo.
- Navegação completa por controle, com teclado virtual.
- Salas privadas com convites temporários; servidor de testes já configurado.
- Modo bandeja, métricas avançadas e log em `%LOCALAPPDATA%\Next Estudios\Vellcast\data\logs`.

Validado em Windows real (CI): transmissão pelo app até o servidor de produção, sincronia A/V de 9–12 ms, bitrate adaptativo (3000 → 500 kbps com banda limitada), modo competitivo (60 → 24 FPS sob carga), sessão de 60 minutos sem vazamento e instalação/desinstalação silenciosas.

### Limitações conhecidas
- O servidor de testes fica na Europa: a partir do Brasil, use a qualidade **Econômico**.
- Ainda não testado com GPUs e jogos reais nem com controle físico; a integração com o Discord aguarda a aplicação no Discord.
- Executável sem assinatura de código.

[Unreleased]: https://github.com/nextestudios/Vellcast/compare/v0.1.2...HEAD
[0.1.2]: https://github.com/nextestudios/Vellcast/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/nextestudios/Vellcast/releases/tag/v0.1.1
