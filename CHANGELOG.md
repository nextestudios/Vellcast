# Histórico de alterações

Notas em português do Brasil; a versão em inglês (Estados Unidos) fica em `CHANGELOG.en-US.md`. Antes de publicar uma versão, adicione uma seção `## [VERSÃO]` **nos dois arquivos** (no PR `chore(release): VERSÃO`, junto com a versão do `Cargo.toml`). O workflow de release usa a seção da tag e falha se faltar alguma. O formato segue o [Keep a Changelog](https://keepachangelog.com/pt-BR/1.1.0/).

## [Unreleased]
### Novidades
- **Nome do convidado e lista de participantes no player web (`/watch`):** quem abre um link de convite informa um nome antes de entrar; o convite só é usado quando o nome é enviado (fechar a página antes não gasta o convite). Depois de entrar, o player mostra quem está na sala e quantos são, atualizando a cada entrada, saída, queda ou reconexão, sem nomes duplicados. O transmissor aparece com o nome de exibição das configurações do app. Nomes são sempre exibidos como texto; convites e tokens nunca aparecem na lista.
- **Relay:** `POST /api/rooms/{id}/join` aceita `name` (1 a 32 caracteres, validado antes de consumir o convite), o nome vai assinado no token de espectador e a nova mensagem `Presence` (`0x40`) envia a lista de participantes aos espectadores ([protocolo](docs/STREAMING_PROTOCOL.md#33-participantes-presence)). Players antigos ignoram a mensagem.
- **Salas pelo Início:** a tela Início ganhou "Criar sala" e "Entrar com link". Cole o link de convite (ou toque em **Colar link**) para entrar na sala de outra pessoa; a sala aparece em Salas marcada como **Participante**.
- **Qualquer participante pode compartilhar a tela:** na sala, toque em **Compartilhar minha tela**, escolha o que compartilhar e inicie. Uma pessoa compartilha por vez: quem começa assume a vez, quem compartilhava é avisado e os espectadores passam para o novo fluxo sem imagem travada. Parar, sair da sala ou cair libera a vez na hora.
- A sala mostra quem está compartilhando (no app e no player web) e tem **Assistir no navegador**, **Parar de compartilhar** e **Sair da sala**.
- **Controles rápidos na tela Ao vivo:** troque o que está compartilhando, a qualidade (conteúdo, resolução, taxa de quadros e taxa de bits), o áudio do jogo e o microfone sem sair da transmissão, com controle, mouse ou teclado. Taxa de bits, preferência de conteúdo e áudio valem na hora; fonte, resolução e taxa de quadros reiniciam a captura e o encoder na mesma sala, depois de uma confirmação que avisa do breve congelamento.
- **Microfone (opcional):** escolha o microfone, ligue, desligue ou silencie durante a transmissão. Ele vem desligado e é misturado ao áudio do jogo, que continua controlado à parte (só o aplicativo, sistema inteiro sem a voz do Discord, ou desligado).
- **Preferência de conteúdo "Texto nítido":** para código e documentos, quando a rede fica lenta o Vellcast reduz a taxa de quadros em vez de borrar o texto.

### Correções
- **Player web cabe na janela:** a página `/watch` ocupa exatamente a área visível do navegador, sem barras de rolagem. A imagem é ajustada com faixas pretas quando a proporção da transmissão difere da janela (sem esticar nem cortar), acompanha redimensionamento e tela cheia, e os controles, a barra de status e as estatísticas continuam visíveis, inclusive em telas estreitas.

### Segurança
- Só quem entrou com um convite válido (ou o dono) pode pedir para compartilhar; revogar um participante encerra o compartilhamento dele. Convites, tokens e nomes não aparecem nos logs do servidor.

### Mudanças
- Se a janela ou o monitor compartilhado fechar, a transmissão não termina mais na hora: os espectadores veem a última imagem e você tem 2 minutos para escolher outra fonte.
- As opções de qualidade mostram só o que o Vellcast e o plano realmente oferecem (até 1080p60 no plano Gratuito); o que não está disponível aparece numa nota.

### Limitações conhecidas
- Microfone real, troca de dispositivo e controle físico ainda não foram validados em hardware; o servidor de testes da CI não tem microfone.

## [0.2.0-alpha.1]
### Novidades
- **Código aberto sob a AGPL-3.0-only:** o Vellcast passa a ser software livre; o repositório [github.com/nextestudios/Vellcast](https://github.com/nextestudios/Vellcast) será aberto quando o projeto estiver pronto. As versões até a 0.1.2 foram distribuídas sob licença proprietária ([notas de licença](docs/LICENSING.md)). Os serviços comerciais (contas e planos) continuam num repositório privado separado e não são necessários para transmitir.
- **Versão portátil:** `Vellcast-Portable-x64.exe`, um único executável que guarda configurações e logs na pasta `Vellcast_Data` ao lado dele. Ele avisa quando há versão nova; a troca do arquivo é manual.

### Mudanças
- **Atualizações entre versões alpha:** o verificador de atualizações agora compara pré-lançamentos pela regra do SemVer (`0.2.0-alpha.1` < `0.2.0-alpha.2` < `0.2.0`).
- **Releases no padrão do ControlFS:** publicadas por tag na `main`, com notas em inglês e português, testes completos e o app aberto de verdade (portátil e instalado) antes de publicar; `SHA256SUMS.txt` com o hash dos dois downloads.
- A tela **Sobre** mostra a nova licença e o endereço do código-fonte.

### Limitações conhecidas
- Ainda não validado com GPUs e jogos reais nem com controle físico; a integração com o Discord aguarda a aplicação no Discord.
- O servidor de testes fica na Europa: a partir do Brasil, use a qualidade **Econômico**.
- Executáveis sem assinatura de código ([política](docs/CODE_SIGNING.md)): o SmartScreen pode avisar na primeira execução.
- Quem está na 0.1.1 ou na 0.1.2 vê esta versão em **Sobre → Verificar atualizações**.

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

[Unreleased]: https://github.com/nextestudios/Vellcast/compare/v0.2.0-alpha.1...HEAD
[0.2.0-alpha.1]: https://github.com/nextestudios/Vellcast/compare/v0.1.2...v0.2.0-alpha.1
[0.1.2]: https://github.com/nextestudios/Vellcast/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/nextestudios/Vellcast/releases/tag/v0.1.1
