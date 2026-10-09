# Matriz MoSCoW — Vellcast

> Fonte de verdade para prioridades do produto. Toda tarefa do [ROADMAP](ROADMAP.md) referencia um ou mais IDs desta matriz.
> Status permitidos: `Backlog`, `Ready`, `In Progress`, `Blocked`, `Testing`, `Done`.
> Nenhum item é marcado `Done` sem cumprir integralmente os critérios de aceite e a [Definição de Pronto](ROADMAP.md#definição-de-pronto).

Última revisão: 2026-10-08 (início da Etapa 0).

## Resumo

| Categoria | Itens | Backlog | In Progress | Blocked | Done |
|---|---|---|---|---|---|
| Must Have | 25 | 21 | 4 | 0 | 0 |
| Should Have | 16 | 16 | 0 | 0 | 0 |
| Could Have | 20 | 20 | 0 | 0 | 0 |
| Won't Have | 17 | — (fora do escopo) | — | — | — |

Atualizado em 2026-10-08: todos os Must do MVP Windows estão implementados e em **Testing** (falta a execução em Windows real); M-012, M-014 e M-015 foram adiados pela ADR-013 e M-025 depende de testes prolongados no Windows.

Observação de ambiente: o desenvolvimento atual ocorre em macOS. Itens que dependem de captura, GPU, áudio WASAPI ou gamepad no Windows só podem ser validados em máquina Windows real; a CI Windows cobre apenas compilação e testes sem hardware.

---

## MUST HAVE

### M-001 — Aplicativo nativo Windows
- **Descrição:** Aplicativo desktop em Rust com Slint como framework de UI, sem Electron ou navegador embutido.
- **Prioridade:** Must · **Etapa:** 1
- **Dependências:** Decisão de licenciamento do Slint (ver [LICENSING](LICENSING.md)).
- **Critérios de aceite:** binário Release único para Windows 11 x64; nenhuma dependência de Electron/WebView; ocioso < 1% CPU e < 80 MB RAM medidos em Release (meta, não garantia).
- **Testes necessários:** build Release na CI Windows; medição de CPU/RAM ocioso em hardware de referência; inventário de dependências.
- **Riscos técnicos:** licença do Slint incompatível com o modelo Open Core; consumo de GPU do renderer do Slint.
- **Status:** Testing — app Rust + Slint compila e roda no macOS; build Release Windows na CI; execução no Windows pendente

### M-002 — Interface amigável
- **Descrição:** UI moderna, minimalista, dark mode, organização familiar a usuários do Discord, com identidade visual própria.
- **Prioridade:** Must · **Etapa:** 1
- **Dependências:** M-001, [UX_GUIDELINES](UX_GUIDELINES.md).
- **Critérios de aceite:** telas Home, Transmitir, Salas, Transmissão ativa, Configurações, Conta, Planos e Sobre implementadas conforme o design system; contraste mínimo WCAG AA; nenhum asset ou layout copiado do Discord.
- **Testes necessários:** revisão visual contra o design system; verificação de contraste; teste de usabilidade com o fluxo principal.
- **Riscos técnicos:** limitações de componentes do Slint; esforço de design subestimado.
- **Status:** Testing — 8 telas com design system próprio; revisão visual no Windows pendente

### M-003 — Suporte completo a controles
- **Descrição:** Todas as funções essenciais acessíveis apenas com gamepad, mantendo mouse e teclado funcionais.
- **Prioridade:** Must · **Etapa:** 1
- **Dependências:** M-001, crate `controller`.
- **Critérios de aceite:** 100% das ações das telas da etapa navegáveis por gamepad; foco sempre visível; modais navegáveis; deadzone configurável; repetição controlada; nenhum input capturado em segundo plano por padrão.
- **Testes necessários:** testes unitários do mapeamento e da máquina de foco; teste manual com Xbox, DualShock 4, DualSense e controle genérico; teste de conexão/desconexão.
- **Riscos técnicos:** variação de drivers; conflito de foco com o jogo em primeiro plano.
- **Status:** Testing — navegação completa por controle validada com ações simuladas; controle físico pendente

### M-004 — Teclado virtual
- **Descrição:** Teclado virtual original, navegável por D-pad e analógico.
- **Prioridade:** Must · **Etapa:** 1
- **Dependências:** M-003.
- **Critérios de aceite:** letras, números, símbolos, espaço, apagar, maiúsculas/minúsculas, confirmar e cancelar; abre automaticamente ao focar campo de texto via gamepad; teclado físico continua funcionando.
- **Testes necessários:** testes unitários de navegação de grade; teste manual com gamepad.
- **Riscos técnicos:** suporte a IME/caracteres acentuados; integração com campos de texto do Slint.
- **Status:** Testing — teclado virtual validado com ações simuladas; controle físico pendente

### M-005 — Captura de janelas
- **Descrição:** Selecionar e capturar uma janela/jogo via Windows Graphics Capture.
- **Prioridade:** Must · **Etapa:** 2
- **Dependências:** crate `capture`, Etapa 0 concluída.
- **Critérios de aceite:** listar janelas capturáveis; capturar com seleção explícita; tratar redimensionamento, fechamento da janela e perda de dispositivo sem crash.
- **Testes necessários:** testes de integração em Windows real (janela comum e jogo DirectX); teste de fechamento e resize.
- **Riscos técnicos:** borda amarela do WGC; jogos em fullscreen exclusivo; diferenças entre builds do Windows.
- **Status:** Testing — WGC de janela implementado e compilado no Windows; execução real pendente

### M-006 — Captura de monitores
- **Descrição:** Compartilhar um monitor completo.
- **Prioridade:** Must · **Etapa:** 2
- **Dependências:** M-005.
- **Critérios de aceite:** listar monitores; capturar monitor escolhido; tratar mudança de resolução/DPI e desconexão de monitor.
- **Testes necessários:** teste em setup multi-monitor; troca de resolução durante captura.
- **Riscos técnicos:** HDR e escalas de DPI mistas; fallback DXGI Desktop Duplication.
- **Status:** Testing — WGC de monitor implementado e compilado no Windows; execução real pendente

### M-007 — Encoder por hardware
- **Descrição:** Codificação H.264 acelerada por GPU com detecção de encoders (NVENC, AMF, Quick Sync).
- **Prioridade:** Must · **Etapa:** 2
- **Dependências:** M-005, crate `encoder`.
- **Critérios de aceite:** detectar encoders disponíveis; codificar texturas D3D11 sem cópia para CPU; modo baixa latência sem B-frames; keyframe sob demanda; recuperação de falhas.
- **Testes necessários:** testes em pelo menos uma GPU NVIDIA, AMD e Intel; teste de falha/reinicialização do encoder.
- **Riscos técnicos:** variação de recursos entre MFTs de fabricantes; drivers antigos.
- **Status:** Testing — Media Foundation, hardware primeiro (software como fallback); execução real pendente

### M-008 — Transmissão 1080p60 gratuita
- **Descrição:** Plano Free permite até 1920x1080 a 60 FPS, sem marca d'água.
- **Prioridade:** Must · **Etapa:** 2 (pipeline) / 5 (fim a fim)
- **Dependências:** M-007, M-010, M-012.
- **Critérios de aceite:** nenhuma restrição de resolução no código aberto; 1080p60 alcançado fim a fim em hardware compatível com métricas registradas.
- **Testes necessários:** teste fim a fim 1080p60 com medição de FPS entregue ao espectador.
- **Riscos técnicos:** capacidade do proxy do Discord para ~8 Mbps; custo de banda do serviço oficial.
- **Status:** Testing — perfil padrão 1080p60/8 Mbps sem assinatura; reprodução 1080p60 validada no player web; captura real pendente

### M-009 — Captura de áudio
- **Descrição:** Capturar áudio do aplicativo selecionado (WASAPI Application Loopback), com fallback para áudio do sistema.
- **Prioridade:** Must · **Etapa:** 4
- **Dependências:** crate `audio`, M-005.
- **Critérios de aceite:** captura por processo quando suportada; fallback com aviso claro; Opus 48 kHz; volume ajustável; não capturar voz do Discord indevidamente.
- **Testes necessários:** teste em Windows 11 (e versões de Windows 10 suportadas); teste de eco com Discord aberto.
- **Riscos técnicos:** disponibilidade do Application Loopback por build; processos com áudio em processo filho.
- **Status:** Testing — WASAPI por processo + Opus; execução real pendente

### M-010 — Servidor de transmissão
- **Descrição:** Servidor Rust (`vellcast-relay`) que recebe e distribui mídia codificada sem recodificar.
- **Prioridade:** Must · **Etapa:** 3
- **Dependências:** crate `protocol`, Etapa 0.
- **Critérios de aceite:** recebe um stream do transmissor e distribui a N espectadores; sem decodificação/recodificação; filas limitadas; Docker de produção.
- **Testes necessários:** testes de integração com múltiplos clientes; teste de carga; teste de espectador lento.
- **Riscos técnicos:** custo de banda; head-of-line blocking no WebSocket.
- **Status:** Testing — `vellcast-relay` implementado (49 testes, benchmark local de 20 espectadores)

### M-011 — Múltiplos espectadores
- **Descrição:** Vários espectadores assistem ao mesmo stream sem codificação separada por espectador.
- **Prioridade:** Must · **Etapa:** 3
- **Dependências:** M-010.
- **Critérios de aceite:** uma codificação, N espectadores; fila independente e limitada por espectador; espectador lento não afeta os demais; novo espectador inicia no próximo keyframe.
- **Testes necessários:** teste com ≥ 10 espectadores simulados, incluindo um lento; medição de memória.
- **Riscos técnicos:** tempo até o primeiro frame para novos espectadores; custo de banda.
- **Status:** Testing — fan-out sem recodificação validado (bytes idênticos); deploy real pendente

### M-012 — Discord Activity
- **Descrição:** Activity para assistir às transmissões dentro do Discord, sem instalar o desktop.
- **Prioridade:** Must · **Etapa:** 0 (viabilidade) / 5 (integração)
- **Dependências:** App registrado no Discord Developer Portal, túnel/host HTTPS, URL Mappings.
- **Critérios de aceite:** Activity inicia via Embedded App SDK; reproduz H.264 via WebCodecs e Opus via AudioDecoder (ou fallback documentado); métricas registradas no Discord Desktop e Web.
- **Testes necessários:** teste em Activity real no Discord Desktop, Discord Web e, quando aplicável, mobile.
- **Riscos técnicos:** suporte a WebCodecs por cliente; throughput do proxy; CSP.
- **Status:** Blocked — adiado por decisão (ADR-013); player web substitui no MVP

### M-013 — Salas privadas
- **Descrição:** Salas privadas com convites e autorização.
- **Prioridade:** Must · **Etapa:** 3 / 5
- **Dependências:** M-014, M-010.
- **Critérios de aceite:** criar, entrar, sair, encerrar, convidar, validar acesso, expirar sessões; privada por padrão.
- **Testes necessários:** testes unitários do gerenciador de salas; testes de autorização negativa.
- **Riscos técnicos:** modelo de identidade entre desktop e Discord.
- **Status:** Testing — salas privadas e convites temporários no relay e na interface

### M-014 — Autenticação
- **Descrição:** Autenticação segura, com identidade Discord (OAuth2 via Embedded App SDK) quando necessário.
- **Prioridade:** Must · **Etapa:** 5
- **Dependências:** crate `auth`, app Discord configurado.
- **Critérios de aceite:** troca de código OAuth2 apenas no backend; client secret em variável de ambiente; tokens de sessão curtos; identidade nunca confiada a partir do frontend.
- **Testes necessários:** testes de token expirado, adulterado e reutilizado.
- **Riscos técnicos:** mudanças na API do Discord; vazamento de tokens em logs.
- **Status:** Backlog — adiado (ADR-013): autenticação por tokens de sala no MVP; OAuth Discord na integração com a Activity

### M-015 — Pareamento
- **Descrição:** Vincular app Windows e sessão Discord por código temporário ou fluxo seguro equivalente.
- **Prioridade:** Must · **Etapa:** 5
- **Dependências:** M-014, M-013.
- **Critérios de aceite:** código de uso único com expiração curta; rate limiting; nenhum compartilhamento de credenciais do Discord.
- **Testes necessários:** expiração, reuso, força bruta (rate limit).
- **Riscos técnicos:** UX do pareamento com gamepad.
- **Status:** Backlog — adiado (ADR-013): link de convite substitui o pareamento no MVP

### M-016 — Controles de transmissão
- **Descrição:** Iniciar, parar, selecionar fonte, configurar áudio, resolução, FPS e bitrate.
- **Prioridade:** Must · **Etapa:** 2 / 5
- **Dependências:** M-005, M-007, M-009.
- **Critérios de aceite:** todas as ações disponíveis via UI e gamepad; mudanças aplicadas sem reiniciar o app (reconfiguração ou reinício controlado do pipeline).
- **Testes necessários:** testes de transição de estado; teste manual de cada controle.
- **Riscos técnicos:** reconfiguração do encoder não suportada por alguns MFTs.
- **Status:** Testing — iniciar, parar, fonte, áudio, resolução, FPS, bitrate e encoder na interface

### M-017 — Modo bandeja
- **Descrição:** Minimizar para a bandeja mantendo a transmissão.
- **Prioridade:** Must · **Etapa:** 1
- **Dependências:** M-001.
- **Critérios de aceite:** ícone na bandeja com iniciar/parar/abrir/sair; sem renderização da UI quando minimizado.
- **Testes necessários:** medição de CPU/GPU minimizado; transmissão contínua por 60 min em bandeja.
- **Riscos técnicos:** integração de bandeja fora do Slint.
- **Status:** Testing — transmissão continua na bandeja; menu "Parar transmissão"; Windows pendente

### M-018 — Gerenciamento de recursos
- **Descrição:** Filas limitadas, descarte de frames antigos, prevenção de consumo ilimitado de memória.
- **Prioridade:** Must · **Etapa:** 2 / 3
- **Dependências:** crates `capture`, `encoder`, `transport`.
- **Critérios de aceite:** toda fila tem limite explícito; descarte documentado; memória estável em sessões de 60 min.
- **Testes necessários:** testes unitários de filas; teste de sobrecarga; soak test.
- **Riscos técnicos:** descarte gerando artefatos até o próximo keyframe.
- **Status:** Testing — filas limitadas em captura, encoder, transporte, relay e player

### M-019 — Reconexão
- **Descrição:** Recuperar conexões transmissor↔relay e espectador↔relay quando possível.
- **Prioridade:** Must · **Etapa:** 3 / 5
- **Dependências:** M-010, crate `transport`.
- **Critérios de aceite:** backoff exponencial com jitter; retomada no próximo keyframe; estado da sala preservado durante janela de graça.
- **Testes necessários:** derrubar conexão/servidor e medir tempo de recuperação.
- **Riscos técnicos:** tokens expirados durante a reconexão.
- **Status:** Testing — reconexão do transmissor (janela de graça) e do player validadas localmente

### M-020 — Segurança do streaming
- **Descrição:** Autenticação e autorização no relay.
- **Prioridade:** Must · **Etapa:** 3 / 5
- **Dependências:** M-014, M-013.
- **Critérios de aceite:** toda conexão autenticada; autorização por sala; limites de tamanho de mensagem, espectadores e taxa; TLS obrigatório em produção.
- **Testes necessários:** testes negativos de autorização; fuzzing do parser do protocolo.
- **Riscos técnicos:** abuso do relay como CDN gratuita.
- **Status:** Testing — tokens HMAC, autenticação pela primeira mensagem, rate limit e limites de tamanho

### M-021 — Self-hosting
- **Descrição:** Servidor básico instalável em infraestrutura própria, sem depender da Vellcast Cloud.
- **Prioridade:** Must · **Etapa:** 3
- **Dependências:** M-010.
- **Critérios de aceite:** imagem Docker + docker-compose de exemplo; guia para VPS Linux e Dokploy; funciona sem nenhuma chamada à Cloud.
- **Testes necessários:** instalação limpa seguindo apenas a documentação.
- **Riscos técnicos:** configuração de TLS e proxy reverso pelo usuário.
- **Status:** Testing — Dockerfile, compose, guia Dokploy e pacote Windows com `iniciar-relay.bat`; build Docker pendente

### M-022 — Documentação
- **Descrição:** Documentação técnica e de instalação em PT-BR.
- **Prioridade:** Must · **Etapa:** contínua
- **Dependências:** —
- **Critérios de aceite:** todos os documentos da seção 32 da especificação existem e são atualizados a cada etapa.
- **Testes necessários:** revisão a cada etapa; links válidos.
- **Riscos técnicos:** documentação desatualizada em relação ao código.
- **Status:** Testing — documentação de arquitetura, protocolo, deploy e guia de testes Windows

### M-023 — Métricas
- **Descrição:** Coletar métricas para verificar desempenho, estabilidade e latência.
- **Prioridade:** Must · **Etapa:** 0 (player) / 2–6
- **Dependências:** crate `metrics`.
- **Critérios de aceite:** métricas de FPS, frames descartados, bitrate, latência e tempo de codificação disponíveis localmente; sem envio de conteúdo transmitido.
- **Testes necessários:** validação das métricas contra medição externa.
- **Riscos técnicos:** overhead da própria coleta.
- **Status:** Testing — métricas no app (Ao vivo), no relay (`/metrics`) e no player

### M-024 — Privacidade
- **Descrição:** Não capturar, armazenar ou transmitir conteúdo sem ação explícita.
- **Prioridade:** Must · **Etapa:** todas
- **Dependências:** —
- **Critérios de aceite:** captura só inicia por ação do usuário; indicador visível de transmissão ativa; nenhuma gravação por padrão; logs sem conteúdo.
- **Testes necessários:** revisão de código; teste de que nada é capturado antes do clique em "Iniciar".
- **Riscos técnicos:** —
- **Status:** Testing — captura só por ação explícita, indicador AO VIVO, nada é gravado

### M-025 — Testes de estabilidade
- **Descrição:** Validar sessões prolongadas, desconexões e sobrecarga.
- **Prioridade:** Must · **Etapa:** 6
- **Dependências:** Etapa 5.
- **Critérios de aceite:** sessão ≥ 60 min sem crash e com memória estável; recuperação de desconexões; comportamento previsível sob sobrecarga.
- **Testes necessários:** soak test, chaos de rede (perda/latência), sobrecarga de GPU.
- **Riscos técnicos:** vazamentos em recursos COM/D3D.
- **Status:** Testing — relay 60 min (20 espectadores; 50 em execução) e transmissor 60 min no Windows real sem vazamento; reavaliar com GPU real (um intervalo isolado de 10 s no runner)

---

## SHOULD HAVE

### S-001 — Ajuste automático de bitrate
- **Descrição:** Adaptar bitrate às condições de rede.
- **Prioridade:** Should · **Etapa:** 6
- **Dependências:** M-007, M-019, métricas de fila/RTT.
- **Critérios de aceite:** bitrate reduz sob congestionamento e recupera com histerese; sem oscilação a cada segundo.
- **Testes necessários:** testes com limitação de banda simulada.
- **Riscos técnicos:** sinal de congestionamento pobre em TCP/WebSocket.
- **Status:** Backlog

### S-002 — Modo competitivo
- **Descrição:** Priorizar o desempenho do jogo, reduzindo carga da transmissão quando necessário.
- **Prioridade:** Should · **Etapa:** 6
- **Dependências:** S-001, M-023.
- **Critérios de aceite:** reduz bitrate → FPS → resolução → descarta frames, com histerese; não eleva prioridade do processo.
- **Testes necessários:** benchmark com/sem modo competitivo em jogos de referência.
- **Riscos técnicos:** heurísticas sem métricas internas do jogo.
- **Status:** Backlog

### S-003 — Detecção automática de jogos
- **Descrição:** Identificar janelas de jogos disponíveis.
- **Prioridade:** Should · **Etapa:** 2
- **Dependências:** M-005.
- **Critérios de aceite:** jogos destacados na lista de fontes por heurística não invasiva (sem injeção).
- **Testes necessários:** teste com lista de jogos de referência.
- **Riscos técnicos:** falsos positivos/negativos.
- **Status:** Backlog

### S-004 — Perfis de qualidade
- **Descrição:** Econômico, Equilibrado, Qualidade, Competitivo, Personalizado.
- **Prioridade:** Should · **Etapa:** 5
- **Dependências:** M-016.
- **Critérios de aceite:** perfis aplicam resolução/FPS/bitrate definidos e documentados.
- **Testes necessários:** testes unitários dos perfis; teste manual.
- **Riscos técnicos:** —
- **Status:** Backlog

### S-005 — Transporte QUIC
- **Descrição:** Avaliar QUIC (Quinn) entre app e relay contra WebSocket.
- **Prioridade:** Should · **Etapa:** 6
- **Dependências:** M-010, crate `transport`.
- **Critérios de aceite:** relatório comparativo (latência, CPU, throughput, reconexão, perda, memória, implantação); decisão registrada em DECISIONS.
- **Testes necessários:** benchmark reproduzível com perda/latência simuladas.
- **Riscos técnicos:** UDP bloqueado em redes; proxies que não encaminham QUIC.
- **Status:** Backlog

### S-006 — Suporte a múltiplos encoders
- **Descrição:** Seleção automática e manual de encoder.
- **Prioridade:** Should · **Etapa:** 2 / 6
- **Dependências:** M-007.
- **Critérios de aceite:** lista de encoders detectados; escolha manual persistida; fallback automático em falha.
- **Testes necessários:** máquina com múltiplas GPUs (iGPU + dGPU).
- **Riscos técnicos:** adaptador da captura diferente do adaptador do encoder.
- **Status:** Backlog

### S-007 — Métricas avançadas
- **Descrição:** CPU, RAM, FPS capturado/codificado, frames descartados, bitrate, latência, tempo de codificação.
- **Prioridade:** Should · **Etapa:** 6
- **Dependências:** M-023.
- **Critérios de aceite:** seção própria (fora da Home); atualização apenas quando visível.
- **Testes necessários:** overhead da tela de métricas medido.
- **Riscos técnicos:** —
- **Status:** Backlog

### S-008 — Atalhos personalizados
- **Descrição:** Atalhos de teclado e comandos opcionais de controle.
- **Prioridade:** Should · **Etapa:** 5
- **Dependências:** M-003.
- **Critérios de aceite:** atalhos globais desativados por padrão; configuráveis; sem conflito com jogos.
- **Testes necessários:** teste com jogo em primeiro plano.
- **Riscos técnicos:** hooks globais detectados como suspeitos por anti-cheat.
- **Status:** Backlog

### S-009 — Interface compacta
- **Descrição:** Modo com apenas controles essenciais.
- **Prioridade:** Should · **Etapa:** 5
- **Dependências:** M-002.
- **Critérios de aceite:** modo compacto navegável por controle.
- **Testes necessários:** teste manual.
- **Riscos técnicos:** —
- **Status:** Backlog

### S-010 — Reconexão automática de controles
- **Descrição:** Restaurar navegação após desconexão do gamepad.
- **Prioridade:** Should · **Etapa:** 1
- **Dependências:** M-003.
- **Critérios de aceite:** reconectar o controle restaura o foco sem reiniciar o app.
- **Testes necessários:** desconectar/reconectar USB e Bluetooth.
- **Riscos técnicos:** drivers Bluetooth.
- **Status:** Backlog

### S-011 — Player web externo
- **Descrição:** Visualizar transmissões no navegador fora do Discord usando o relay.
- **Prioridade:** Should · **Etapa:** 5
- **Dependências:** M-010, player da Activity.
- **Critérios de aceite:** mesmo player reaproveitado; autorização igual à Activity; resultados de viabilidade registrados.
- **Testes necessários:** Chrome, Edge, Firefox, Safari.
- **Riscos técnicos:** suporte desigual a WebCodecs entre navegadores.
- **Status:** Backlog

### S-012 — Atualizações
- **Descrição:** Sistema de atualização seguro sem serviços residentes pesados.
- **Prioridade:** Should · **Etapa:** 8
- **Dependências:** S-016.
- **Critérios de aceite:** pacotes assinados; verificação de assinatura antes de aplicar; sem serviço permanente.
- **Testes necessários:** atualização, rollback, pacote adulterado.
- **Riscos técnicos:** custo de certificado de assinatura de código.
- **Status:** Backlog

### S-013 — Localização
- **Descrição:** Internacionalização com PT-BR e inglês.
- **Prioridade:** Should · **Etapa:** 1 (preparação) / 8
- **Dependências:** M-002.
- **Critérios de aceite:** nenhuma string de UI hardcoded; troca de idioma funcional.
- **Testes necessários:** verificação de strings não traduzidas.
- **Riscos técnicos:** —
- **Status:** Backlog

### S-014 — Observabilidade
- **Descrição:** Métricas agregadas de capacidade do servidor e diagnóstico de falhas.
- **Prioridade:** Should · **Etapa:** 3 / 8
- **Dependências:** M-010.
- **Critérios de aceite:** endpoint de métricas e health check; logs estruturados sem dados sensíveis.
- **Testes necessários:** teste de carga observando métricas.
- **Riscos técnicos:** —
- **Status:** Backlog

### S-015 — Estrutura comercial
- **Descrição:** Backend preparado para contas, planos, cotas, assinaturas e entitlements.
- **Prioridade:** Should (MVP) / Must (lançamento Premium) · **Etapa:** 7
- **Dependências:** M-014, decisão sobre provedor de pagamento e políticas do Discord.
- **Critérios de aceite:** entitlements validados no backend; núcleo de streaming independente do billing.
- **Testes necessários:** testes de entitlements e webhooks com provedor em modo de teste.
- **Riscos técnicos:** elegibilidade de Premium Apps no Brasil; obrigações fiscais.
- **Status:** Backlog

### S-016 — Instalador
- **Descrição:** Instalador Windows com atualização e desinstalação simples.
- **Prioridade:** Should · **Etapa:** 8
- **Dependências:** M-001.
- **Critérios de aceite:** instalação sem privilégios de administrador quando possível; desinstalação limpa; divulgação de licenças.
- **Testes necessários:** instalação/desinstalação em Windows limpo; SmartScreen.
- **Riscos técnicos:** reputação SmartScreen sem assinatura de código.
- **Status:** Backlog

---

## COULD HAVE

Não implementar antes do núcleo (Must) estar estável.

| ID | Título | Etapa-alvo | Observação | Status |
|---|---|---|---|---|
| C-001 | Transmissão 1440p60 | Pós-MVP (Pro) | Depende de banda/custo e suporte do decoder | Backlog |
| C-002 | Transmissão 4K60 | Pós-MVP (Pro) | Exige validação de decode na Activity | Backlog |
| C-003 | Codec AV1 | Pós-MVP | Suporte de encoder (RTX 40+, RX 7000, Arc) e decoder variável | Backlog |
| C-004 | Codec H.265 | Pós-MVP | Suporte WebCodecs limitado e questões de patentes | Backlog |
| C-005 | Transmissão 120 FPS | Pós-MVP | Custo de banda e capacidade do proxy | Backlog |
| C-006 | Temas personalizados | Pós-MVP | Sem impacto em desempenho | Backlog |
| C-007 | Salas públicas | Pós-MVP | Requer moderação e proteção contra abuso | Backlog |
| C-008 | Descoberta de transmissões | Pós-MVP | Depende de C-007 | Backlog |
| C-009 | Gravação local | Pós-MVP | Nunca por padrão (W-008) | Backlog |
| C-010 | Picture-in-picture | Pós-MVP | Depende do suporte da Activity | Backlog |
| C-011 | Várias transmissões em uma sala | Pós-MVP | Multiplica banda por espectador | Backlog |
| C-012 | Gerenciamento avançado de comunidades | Pós-MVP (Community) | Depende de S-015 | Backlog |
| C-013 | Dashboard web administrativo | Pós-MVP | Painel mínimo previsto na Etapa 7 | Backlog |
| C-014 | Servidores de mídia regionais | Pós-MVP | Depende de métricas de uso | Backlog |
| C-015 | Aplicativo Linux | Pós-MVP | Captura via PipeWire | Backlog |
| C-016 | Aplicativo macOS | Pós-MVP | Captura via ScreenCaptureKit | Backlog |
| C-017 | Player mobile independente | Pós-MVP | Avaliar após S-011 | Backlog |
| C-018 | Sistema de plugins | Pós-MVP | Superfície de ataque a avaliar | Backlog |
| C-019 | Integrações adicionais | Pós-MVP | — | Backlog |
| C-020 | Modo P2P opcional fora do Discord | Pós-MVP | Nunca como transporte principal da Activity (W-017) | Backlog |

---

## WON'T HAVE (fora do escopo atual)

WON'T HAVE significa fora do escopo atual, não necessariamente proibição permanente.

| ID | Título | Motivo | Status |
|---|---|---|---|
| W-001 | Electron | Consumo de recursos incompatível com a proposta | Fora do escopo |
| W-002 | Navegador completo embutido no transmissor | Mesmo motivo de W-001 | Fora do escopo |
| W-003 | Chat de voz próprio | Voz permanece no Discord | Fora do escopo |
| W-004 | Substituição completa do Discord | Produto especializado em compartilhamento | Fora do escopo |
| W-005 | WebRTC dentro da Discord Activity | Não suportado pelo proxy do Discord | Fora do escopo |
| W-006 | Recodificação obrigatória no servidor | Custo de CPU/GPU e latência | Fora do escopo |
| W-007 | Captura escondida de tela | Princípio de segurança/consentimento | Proibido |
| W-008 | Gravação automática de transmissões | Privacidade | Proibido |
| W-009 | Animações gráficas pesadas | Desempenho | Fora do escopo |
| W-010 | Mineração, publicidade invasiva ou processamento oculto | Ética e desempenho | Proibido |
| W-011 | Injeção em processos de jogos | Estabilidade e anti-cheat | Proibido |
| W-012 | Integração que interfira com anti-cheat | Risco de banimento de usuários | Proibido |
| W-013 | Dependência obrigatória da Vellcast Cloud para self-hosting | Princípio Open Core | Proibido |
| W-014 | Bloqueio de resolução no código aberto | Princípio Open Core | Proibido |
| W-015 | Mecanismo próprio de DRM | Fora da proposta | Fora do escopo |
| W-016 | Aplicativo multiplataforma no primeiro MVP | Foco em Windows 11 | Fora do escopo |
| W-017 | P2P como transporte principal da integração Discord | Arquitetura cliente-servidor | Fora do escopo |
