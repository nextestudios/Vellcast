# Testando o MVP no Windows

Guia para validar o Vellcast em uma máquina Windows real: captura, encoder por hardware, áudio, controle e distribuição para espectadores no navegador.

> A integração com o Discord (Activity, OAuth, pareamento) foi adiada pelo ADR-013. Neste MVP, quem assiste usa o **player web** servido pelo próprio relay, por um link de convite.

## 1. Requisitos

| Item | Mínimo | Observação |
|---|---|---|
| Sistema | Windows 10 versão 2004 (maio/2020) | Windows 11 recomendado. Captura por janela e áudio por processo dependem dessas versões. |
| GPU | Qualquer com encoder H.264 | NVIDIA (NVENC), AMD (VCE/VCN) ou Intel (Quick Sync). Sem encoder de hardware, o app usa o encoder de software do Windows (mais CPU). |
| Quem assiste | Chrome, Edge ou outro Chromium recente | Precisa de WebCodecs (H.264 + Opus). Firefox e Safari: não testados. |
| Rede | Upload ≥ 10 Mbps para 1080p60 a 8 Mbps | Cada espectador consome a banda de saída do **relay**, não a do PC que transmite. |

## 2. Obter o pacote

1. No GitHub, abra **Actions → Windows MVP** e escolha a execução mais recente da `main` (ou use **Run workflow**).
2. Baixe o artefato **Vellcast-MVP-windows-x64** e extraia em uma pasta (ex.: `C:\Vellcast`).
3. O pacote não é assinado. O SmartScreen pode avisar: **Mais informações → Executar assim mesmo**.

Conteúdo: `vellcast-desktop.exe`, `vellcast-relay.exe`, `iniciar-relay.bat`, `player\`, `LEIA-ME.txt`, `LICENSE.txt`, `THIRD-PARTY-NOTICES.txt`, `VERSAO.txt`.

## 3. Primeiro teste (tudo no mesmo PC)

1. Execute `iniciar-relay.bat`. Na primeira vez ele cria `relay.env` com um segredo aleatório. Se o Firewall perguntar, permita em redes privadas.
2. Abra `vellcast-desktop.exe`.
3. **Configurações → Servidor**: endereço `http://127.0.0.1:8788` → **Testar conexão** → "Servidor respondeu".
4. **Salas → Criar sala** → selecione a sala → **Gerar link de convite** → **Copiar link**.
5. **Transmitir**:
   - escolha a fonte (jogos prováveis aparecem primeiro, com o selo "Jogo");
   - perfil **Equilibrado** (1080p60, 8 Mbps) ou outro;
   - áudio **Só o aplicativo escolhido**;
   - sala de destino → **Iniciar transmissão**.
6. O app vai para **Ao vivo**, com o indicador **AO VIVO** na janela, na barra de status e na dica do ícone da bandeja.
7. Cole o link no Chrome/Edge do mesmo PC. A imagem deve aparecer em até ~1 s. Clique em **Ativar áudio**.
8. **Parar transmissão** (na tela Ao vivo ou no menu da bandeja). O player mostra "Aguardando transmissão".

## 4. Assistir em outro aparelho

O navegador só decodifica vídeo em **HTTPS** (ou `localhost`). Um link `http://192.168.x.x` abre a página, mas o vídeo não toca.

**Opção A — túnel HTTPS temporário** (mais rápido para testar):

1. Instale o [cloudflared](https://developers.cloudflare.com/cloudflare-one/connections/connect-networks/downloads/) e rode `cloudflared tunnel --url http://localhost:8788`.
2. Copie o endereço `https://….trycloudflare.com` exibido.
3. No desktop: **Configurações → Servidor → Endereço público para os links de convite** = esse endereço. O servidor continua `http://127.0.0.1:8788`.
4. Gere um **novo** link de convite e abra-o no celular ou em outro PC.

**Opção B — VPS com TLS** (uso contínuo): siga `docs/DEPLOYMENT.md` §6.4–6.5 (Docker + Dokploy + Let's Encrypt). No desktop, use `https://seu-dominio` como servidor; o app transmite por `wss://`.

## 5. Checklist dos requisitos Must Have

Registre o resultado de cada linha (OK / Falhou / Não testado) e o hardware usado.

| ID | O que verificar | Como | Resultado esperado |
|---|---|---|---|
| M-001 | App nativo, sem Electron | Gerenciador de Tarefas → Detalhes | Um processo `vellcast-desktop.exe`, sem processos de navegador |
| M-002 | Interface amigável | Navegar pelas 8 telas | Tema escuro, foco amarelo visível, textos em pt-BR (e en em Configurações) |
| M-003 | Navegação por controle | Usar só o gamepad (D-pad/analógico, A, B, LB/RB, Start) | Todas as telas e ações acessíveis; com o app fora de foco, o controle não age no app |
| M-004 | Teclado virtual | Com o controle, confirmar em "Nome da sala" | Teclado aparece; digitar, apagar, maiúsculas, confirmar e cancelar funcionam |
| M-005 | Captura de janela | Transmitir uma janela de jogo | Imagem da janela no player; mover e redimensionar a janela não derruba a transmissão |
| M-006 | Captura de monitor | Transmitir um monitor | Monitor inteiro no player |
| M-007 | Encoder por hardware | Ao vivo → Vídeo; Gerenciador de Tarefas → GPU → "Video Encode" | Nome do encoder de hardware; uso de "Video Encode" na GPU, CPU baixa |
| M-008 | 1080p60 grátis | Perfil Equilibrado | Ao vivo mostra 1920×1080 @ 60; o player mostra ~60 FPS |
| M-009 | Áudio do aplicativo | Áudio "Só o aplicativo escolhido" + vídeo/jogo com som | Som no player após "Ativar áudio"; outros apps (ex.: Discord) não aparecem |
| M-010 | Servidor de transmissão | `http://127.0.0.1:8788/metrics` durante a transmissão | Contadores de mensagens e bytes crescendo |
| M-011 | Vários espectadores | Abrir o link em 3 abas/aparelhos (o link vale para 10 pessoas) | Todos assistem; a CPU do PC que transmite não aumenta por espectador |
| M-012 | Discord Activity | — | **Adiado** (ADR-013) |
| M-013 | Salas privadas | Abrir `/watch` sem convite e com convite já usado | Acesso negado com mensagem clara; só links válidos assistem |
| M-014 | Autenticação | Encerrar a sala com espectadores conectados | Espectadores veem "sala encerrada"; links antigos param de funcionar |
| M-015 | Pareamento | — | **Adiado** com o Discord (ADR-013); o link de convite cumpre o papel no MVP |
| M-016 | Controles de transmissão | Iniciar, parar, fonte, áudio, resolução, FPS, bitrate (perfil Personalizado) | Valores aplicados aparecem em Ao vivo → Vídeo |
| M-017 | Modo bandeja | Fechar a janela durante a transmissão | Transmissão continua; ícone na bandeja com dica "AO VIVO"; menu "Parar transmissão" funciona |
| M-018 | Recursos limitados | Transmitir 30 min; observar memória no Gerenciador de Tarefas | Memória estável (sem crescimento contínuo) |
| M-019 | Reconexão | Com o relay em outra máquina, VPS ou túnel: desligue a rede do PC que transmite por ~10 s e religue (menos de 30 s). No espectador, recarregue a página (F5) | Ao vivo mostra "Reconectando…" e volta; o player mostra "o transmissor está reconectando" e retoma sozinho; após F5 o player volta sem novo convite. Obs.: reiniciar o relay apaga as salas (estado em memória, E3-T11) |
| M-020 | Segurança do streaming | Tentar `/ws` sem token; inspecionar a URL do player após abrir o link | Conexão recusada; o código do convite some da barra de endereços |
| M-021 | Self-hosting | Rodar o relay em outra máquina ou VPS | Funciona sem conta ou assinatura |
| M-022 | Documentação | Seguir só este guia e o LEIA-ME | Teste concluído sem ajuda externa |
| M-023 | Métricas | Ao vivo → "Saúde da transmissão"; player → Estatísticas | FPS capturado/codificado, bitrate, descartes, tempo de codificação, latência |
| M-024 | Privacidade | Abrir o app e não clicar em Iniciar | Nada é capturado; o indicador AO VIVO só aparece após Iniciar |
| M-025 | Estabilidade | Sessão de 60 min com 2+ espectadores | Sem queda; descartes baixos; memória estável |

## 6. Coletando números de desempenho

Para o relatório em `docs/PERFORMANCE.md`, registre hardware (CPU, GPU, driver), versão do Windows, rede e o jogo usado.

- **Impacto no jogo:** FPS médio e 1% lows com e sem transmissão (overlay do jogo, PresentMon, CapFrameX ou o overlay do driver da GPU).
- **Desktop:** Gerenciador de Tarefas → Detalhes → `vellcast-desktop.exe` (CPU, memória — "Conjunto de trabalho privado") e GPU → "Video Encode".
- **Transmissão:** tela Ao vivo (FPS capturado/codificado, bitrate, descartes, tempo de codificação, latência até o servidor).
- **Espectador:** player → Estatísticas (FPS, bitrate, atraso p95, descartes).
- **Relay:** `http://<relay>/metrics`.
- **Latência ponta a ponta (opcional):** transmita um cronômetro na tela e fotografe a fonte e o player lado a lado.

## 7. Onde ficam os dados do app

- Configurações: `%APPDATA%\Next Estudios\Vellcast\config\settings.json`.
- Salas criadas: `%APPDATA%\Next Estudios\Vellcast\config\rooms.json`. Contém o token de dono de cada sala; não compartilhe esse arquivo.
- Segredo do relay do pacote: `relay.env` na pasta do pacote. Trocar o segredo invalida todas as salas e links.
- Logs: execute `vellcast-desktop.exe` a partir de um terminal com `set RUST_LOG=debug` para ver os logs. Tokens nunca são registrados.

## 8. Reportando problemas

Abra uma issue (repositório privado) com:

- Windows, GPU e driver, e `VERSAO.txt` do pacote;
- passo a passo, resultado esperado e resultado obtido;
- prints da tela Ao vivo e das Estatísticas do player;
- saída do `iniciar-relay.bat` e, se possível, os logs do desktop com `RUST_LOG=debug`.

Nunca inclua o conteúdo de `relay.env`, `rooms.json` ou links de convite válidos.
