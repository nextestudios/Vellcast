# Registro de decisões de arquitetura (ADR)

Formato: contexto → decisão → consequências. Status possíveis: **Proposta**, **Aceita**, **Substituída**, **Rejeitada**.

---

## ADR-001 — Arquitetura cliente-servidor com relay sem recodificação
**Status:** Aceita
**Contexto:** Vários espectadores precisam assistir a uma única transmissão, e a Activity só permite WebSocket. P2P não escala com o upload do transmissor e não é viável dentro da Activity.
**Decisão:** O transmissor envia um único stream codificado ao `vellcast-relay`, que faz fan-out dos mesmos bytes para os espectadores autorizados. Recodificar não faz parte do fluxo normal.
**Consequências:** O servidor não precisa de GPU e o custo é dominado por banda. Não há adaptação de qualidade por espectador, e sim um único nível. Simulcast pode ser avaliado no futuro.

## ADR-002 — WebSocket binário como transporte inicial; QUIC em avaliação
**Status:** Aceita (WS) · Proposta (QUIC)
**Contexto:** A documentação oficial de networking de Activities indica suporte somente a WebSocket. WebRTC não é suportado e WebTransport ainda não.
**Decisão:** Usar WS binário com TLS entre relay e Activity. Entre desktop e relay, começar também com WS e comparar com QUIC (Quinn) na ETAPA 6 (S-005).
**Consequências:** O TCP traz head-of-line blocking sob perda, mitigado com descarte de frames antigos e pedido de keyframe. QUIC exige UDP exposto e não passa por qualquer proxy.

## ADR-003 — Protocolo binário próprio v1 com cabeçalho fixo de 28 bytes
**Status:** Aceita
**Contexto:** O formato precisa ser simples de parsear em Rust e em `DataView` no navegador, versionado e com metadados para sincronia e latência.
**Decisão:** Cabeçalho little-endian: `version u8 | type u8 | flags u16 | stream_id u32 | sequence u32 | pts_us u64 | sent_at_us u64` + payload. Mensagens Init em JSON e mídia em binário. Limite de 4 MiB por mensagem. A especificação está em [STREAMING_PROTOCOL.md](STREAMING_PROTOCOL.md).
**Consequências:** 28 bytes de overhead por frame é desprezível perto do vídeo. Mudanças incompatíveis exigem incrementar `version`.

## ADR-004 — H.264 Annex B com SPS/PPS repetidos antes de cada IDR
**Status:** Aceita
**Contexto:** Novos espectadores entram no meio da transmissão. O WebCodecs aceita Annex B quando `description` é omitido.
**Decisão:** Cada `VideoFrame` carrega uma unidade de acesso completa em Annex B. Cada IDR é precedido por SPS/PPS. Sem B-frames.
**Consequências:** Um espectador pode começar em qualquer keyframe sem estado extra no relay. O custo é o pequeno overhead de repetir SPS/PPS.

## ADR-005 — Fan-out com `tokio::sync::broadcast` limitado e ressincronização por keyframe
**Status:** Aceita
**Contexto:** Um espectador lento não pode bloquear os demais, e a memória precisa ser limitada.
**Decisão:** Um `broadcast` com capacidade fixa por stream e payload `bytes::Bytes` compartilhado. Quando um receptor recebe `Lagged`, ele descarta mensagens até o próximo keyframe.
**Consequências:** A memória fica limitada por stream, não por espectador. Um espectador lento perde trechos em vez de acumular atraso.

## ADR-006 — Licença de distribuição da UI Slint
**Status:** Aceita (2026-10-08): **Royalty-free 2.0**
**Contexto:** O Slint é oferecido sob GPLv3, sob licença royalty-free com atribuição obrigatória (desktop) ou sob licença comercial. Vincular sob GPLv3 impõe GPLv3 ao binário distribuído.
**Opções:** (a) Royalty-free com atribuição: compatível com app desktop gratuito, com a obrigação de exibir a atribuição; é preciso confirmar as condições atuais. (b) GPLv3: o app desktop inteiro passa a ser GPLv3. (c) Comercial: tem custo, mas elimina restrições.
**Decisão:** Royalty-free 2.0, texto lido em 2026-10-08. A atribuição é cumprida com o widget `AboutSlint` na tela "Sobre" e com o badge na página de download. Condições detalhadas em [LICENSING.md](LICENSING.md) §2.
**Consequências:** Sem custo e compatível com o código próprio Apache-2.0 e com recursos pagos. Embarcados ficam fora. O app não pode expor APIs do Slint (vale para plugins futuros).

## ADR-007 — Apache-2.0 para o código próprio aberto
**Status:** Proposta (o arquivo LICENSE foi adicionado; **falta confirmação do proprietário**)
**Contexto:** O modelo é Open Core. A licença precisa ser permissiva e ter concessão explícita de patentes.
**Decisão:** Apache-2.0 para os crates, o relay básico e a Activity. Em relação ao MIT, ela acrescenta concessão de patentes e cláusula de retaliação.
**Consequências:** Fica compatível com uso comercial pela Vellcast Cloud. A compatibilidade com componentes GPL precisa de atenção (ver ADR-006).

## ADR-008 — Banco de dados: SQLite no self-hosting, PostgreSQL avaliado para a Cloud; sem Redis
**Status:** Proposta
**Contexto:** Self-hosters precisam de zero operação. Salas e streams são estado efêmero.
**Decisão:** O estado de mídia e de salas fica em memória no relay. A persistência futura (contas, convites permanentes) vai para SQLite no self-hosting e PostgreSQL na Cloud. Redis só entra com necessidade medida.
**Consequências:** Um relay único não tem alta disponibilidade de salas, o que é aceitável no MVP.

## ADR-009 — Activity servida no mesmo domínio do relay
**Status:** Aceita
**Contexto:** Requisições a hosts não mapeados são bloqueadas pela CSP. Cada mapeamento extra adiciona configuração.
**Decisão:** URL Mapping `/` → host do relay, que serve os estáticos e `/ws`. `patchUrlMappings` só será usado se surgirem hosts adicionais.
**Consequências:** Um único deploy, e a configuração no Developer Portal fica mínima.

## ADR-010 — Latência da ETAPA 0 medida do envio pelo servidor até o frame decodificado
**Status:** Aceita
**Contexto:** A ETAPA 0 usa mídia pré-codificada, sem captura e sem encoder.
**Decisão:** Sincronizar o relógio via Ping/Pong no estilo NTP, escolhendo a amostra de menor RTT, e medir `t_decodificado − sent_at_us`. A métrica é documentada como **parcial**.
**Consequências:** O valor não é comparável com a meta de 500 ms ponta a ponta. A medição completa (captura → tela) fica para a ETAPA 5.

## ADR-011 — Servidor da ETAPA 0 separado e descartável
**Status:** Aceita
**Contexto:** A ETAPA 0 precisa de um servidor rápido para validar a Activity, antes de existir o relay real.
**Decisão:** `services/stage0-server` (`vellcast-stage0-server`) lê mídia sintética gerada por ffmpeg, a ritma em tempo real e distribui pelo protocolo v1. Ele usa `vellcast-protocol`, que será reaproveitado pelo relay.
**Consequências:** O código do servidor pode ser descartado depois, enquanto o protocolo e os aprendizados ficam. Ele não deve ser usado em produção.

## ADR-012 — Mídia de teste gerada localmente, fora do repositório
**Status:** Aceita
**Decisão:** Um script gera vídeo sintético (testsrc2 + bipes de áudio) em H.264 Annex B e Opus. Os arquivos gerados ficam fora do git.
**Consequências:** Os testes são reproduzíveis sem material protegido. O ffmpeg é necessário apenas para desenvolvimento.

## ADR-013 — Validação dentro do Discord adiada; demais Must Have avançam
**Status:** Aceita (2026-10-08, decisão do responsável pelo projeto)
**Contexto:** A ETAPA 0 foi validada fora do Discord (Chromium local: 1080p60 e 720p30 decodificados, áudio Opus, reconexão, 20 espectadores). O teste em uma Activity real (E0-T10) depende de aplicação no Developer Portal e URL HTTPS pública.
**Decisão:** A integração com o Discord fica para depois. E0-T10 volta ao Backlog e as próximas etapas Must Have avançam pela ordem MoSCoW, sem esperar essa validação.
**Consequências:** Assume-se o risco de o Discord não suportar `VideoDecoder`/`AudioDecoder` ou o throughput necessário (R-01, R-03 em `RISKS.md`). O impacto fica limitado ao player da Activity: desktop, captura, encoder e relay continuam válidos, e o player web externo (S-011) usa o mesmo protocolo. E0-T10 passa a ser pré-requisito da ETAPA 5 (integração completa).

## ADR-014 — Desktop: Slint com backend winit e renderer de software
**Status:** Aceita (2026-10-08), a revalidar no Windows
**Contexto:** A meta é UI ociosa abaixo de 1% de CPU e 80 MB de RAM, sem competir com o jogo pela GPU. Opções do Slint: femtovg (OpenGL), Skia (C++, binário maior) e renderer de software com atualização parcial de regiões.
**Medição (macOS, Apple M1, build Release, 20–30 s ocioso, `phys_footprint`):** software 32 MB com janela visível e 20 MB oculto na bandeja; femtovg 41 MB visível. CPU média de 0,03% a 0,27% de um núcleo em todos os casos. Metodologia em `apps/desktop/README.md`.
**Decisão:** Renderer de software como padrão (feature `renderer-software`). O femtovg fica atrás da feature `gpu-renderer` (`SLINT_BACKEND=winit-femtovg`) para benchmarks. Skia descartado por tamanho e toolchain C++.
**Consequências:** Sem contexto de GPU no app e menos memória. A UI é estática e só redesenha regiões alteradas, mas o custo de redesenho em janelas 4K/HiDPI ainda precisa ser medido no Windows; se ficar caro, a decisão volta para femtovg.

## ADR-015 — Navegação por controle sobre a cadeia de foco do Slint
**Status:** Aceita (2026-10-08)
**Contexto:** Todas as funções precisam funcionar só com gamepad (M-003), sem quebrar mouse e teclado.
**Decisão:** O controle gera teclas sintéticas na janela: Baixo/Cima viram Tab/Shift+Tab (foco em ordem), Esquerda/Direita viram setas (ajustam valores em seletores e controles numéricos), A vira Enter. LB/RB trocam de seção, LT/RT trocam de aba, B volta pelo histórico de telas e Start abre o menu rápido. Com um modal aberto (teclado virtual, menu), as ações vão direto ao modal, que mantém o foco preso. Os componentes são próprios (`ui/widgets.slint`) e todos desenham anel de foco.
**Consequências:** Teclado físico e controle percorrem exatamente o mesmo caminho, sem um segundo modelo de foco. A navegação é em ordem, não espacial; navegação espacial fica para avaliar se testes com usuários pedirem.

## ADR-016 — Entrada de gamepad: GilRs, só com a janela em foco
**Status:** Aceita (2026-10-08); validação em hardware pendente
**Decisão:** O GilRs roda numa thread própria, bloqueado em `next_event_blocking` (acorda só por entrada ou prazo de repetição). A thread só emite ações quando a janela do Vellcast está em foco e visível, a partir do evento `Focused` do winit. Em segundo plano, nenhuma ação é gerada e o estado pressionado é descartado. Atalhos globais existem só como configuração, desligada e bloqueada na UI até haver implementação. XInput direto será avaliado apenas se os testes em hardware mostrarem lacunas no GilRs (ex.: gatilhos analógicos ou controles de portáteis).
**Consequências:** O controle nunca interfere no jogo. O evento de foco depende da API `unstable-winit-030` do Slint, por isso a versão do Slint fica fixada com `=`.

## ADR-017 — Internacionalização com traduções embutidas no binário
**Status:** Aceita (2026-10-08)
**Decisão:** Strings da UI com `@tr()`, idioma-fonte inglês e catálogo `pt-BR` em `apps/desktop/translations/pt-BR/LC_MESSAGES/vellcast-desktop.po`, embutido no build (`with_bundled_translations`). O padrão é pt-BR e a troca acontece ao vivo pelo `select_bundled_translation`. Um teste falha se alguma string `@tr` não tiver tradução pt-BR.
**Consequências:** Sem gettext nem arquivos `.mo` em tempo de execução. O inglês é a fonte porque o Slint trata o idioma-fonte como `en`.


## ADR-023 — Áudio: WASAPI Application Loopback com exclusão do Discord
**Status:** Aceita (2026-10-08)
**Contexto:** O áudio transmitido deve ser o do jogo (M-009) sem retransmitir a chamada de voz do Discord (spec §17). O Windows 10 2004+ oferece loopback por processo (`VAD\Process_Loopback`), que inclui **ou** exclui uma única árvore de processos.
**Decisão:** Janela selecionada → loopback **incluindo** a árvore do processo da janela. Modo sistema → loopback **excluindo** a árvore do Discord (raiz detectada por nome de executável e PID pai); sem Discord aberto, exclui o próprio Vellcast. Sem suporte a loopback por processo, o modo sistema cai para o loopback clássico do dispositivo padrão e emite aviso de que a voz do Discord pode ser capturada. Para `AudioTarget::Process`, a indisponibilidade é devolvida como erro (`ProcessLoopbackUnavailable`) para o núcleo decidir o fallback e avisar o usuário.
**Consequências:** Só um processo pode ser excluído: com dois clientes Discord abertos (ex.: estável + Canary), um deles pode ser capturado (há aviso). No fallback clássico não há como excluir o Discord.

## ADR-024 — Áudio em 48 kHz estéreo f32, blocos de 20 ms no relógio QPC
**Status:** Aceita (2026-10-08)
**Decisão:** Pedir ao WASAPI 48 kHz estéreo `f32` com `AUTOCONVERTPCM | SRC_DEFAULT_QUALITY`; se o dispositivo recusar (loopback clássico), usar o formato de mixagem e converter em Rust (downmix por máscara de canais, LFE descartado, reamostragem linear). Os pacotes usam o `qpcPosition` do `GetBuffer` como PTS, relativo à mesma origem QPC do vídeo. Um montador de blocos garante PTS monotônicos e contíguos: lacunas viram silêncio (até 2 s; acima disso a linha do tempo recomeça), sobreposições são cortadas e jitter de até 10 ms é ignorado. Durante silêncio sem pacotes, a linha do tempo é completada até `agora − 100 ms`.
**Consequências:** A/V alinhado por construção, sem acúmulo de buffer. Deriva lenta entre o relógio do dispositivo e o QPC é corrigida em degraus de ≥ 10 ms (pequeno corte ou silêncio). A reamostragem linear é de qualidade de transmissão, não de masterização; é usada só no fallback.

## ADR-025 — Opus via crate `opus` (libopus), 128 kbps, modo Audio
**Status:** Aceita (2026-10-08)
**Contexto:** Encoders Opus em Rust puro existem (ex.: `rusty-opus`, criado em 2026-07), mas são recentes. A libopus é a referência, com licença BSD-3-Clause.
**Decisão:** `opus` 0.4.0 (MIT/Apache-2.0), que compila a libopus estaticamente via `opusic-sys` (cmake), só no Windows. 20 ms por pacote, 48 kHz estéreo, 128 kbps VBR, `Application::Audio`. A feature `opus` (padrão) pode ser desligada só para verificação cruzada sem toolchain C; sem ela a captura falha ao iniciar, em vez de transmitir algo diferente.
**Consequências:** Build Windows precisa de cmake (presente nos runners do GitHub e no Visual Studio). Obrigação de manter o aviso BSD da libopus na distribuição (`LICENSING.md`).

## ADR-018 — Captura WGC com o crate `windows`, sem `windows-capture`
**Status:** Aceita (2026-10-08)
**Contexto:** O crate `windows-capture` (MIT) encapsula o WGC e é popular, mas traz encoder e gravação próprios e esconde parte do controle sobre o pool de quadros e o tempo de vida das texturas, que o pipeline precisa para não copiar quadros.
**Decisão:** `vellcast-capture` usa o `windows` 0.62.2 (o mesmo já presente no `Cargo.lock`) direto: `Direct3D11CaptureFramePool::CreateFreeThreaded` (2 buffers), `GraphicsCaptureSession` com cursor configurável e `IsBorderRequired = false` quando o sistema permitir, evento `Closed` para fonte perdida. O `Direct3D11CaptureFrame` fica vivo dentro do `CapturedFrame` até o encoder consumir a textura.
**Consequências:** Mais código `unsafe` (documentado com `SAFETY:`), em troca de zero cópia e nenhuma dependência extra. Em versões do Windows sem `IsBorderRequired`, a borda amarela do sistema aparece na janela capturada.

## ADR-019 — Conversão BGRA → NV12 e escala na GPU com `ID3D11VideoProcessor`
**Status:** Aceita (2026-10-08)
**Contexto:** Encoders H.264 de hardware esperam NV12. Converter na CPU exigiria ler cada quadro da GPU (proibido pelo princípio 3.1).
**Decisão:** Um `ID3D11VideoProcessor` converte e redimensiona direto para um rodízio de 4 texturas NV12 de tamanho fixo (definido no início: a fonte cabe em `max_width`×`max_height`, sem ampliar, dimensões pares). Se a janela muda de tamanho durante a transmissão, o conteúdo é encaixado com barras pretas (letterbox) em vez de recriar o encoder. Entrada RGB de faixa completa; saída BT.709 de faixa de estúdio.
**Consequências:** Mudança de tamanho não interrompe o fluxo nem exige novo `VideoInit`. Uma janela que cresce muito depois de iniciada não ganha resolução até reiniciar a transmissão.

## ADR-020 — Encoder Media Foundation: hardware primeiro, software Microsoft como último recurso
**Status:** Aceita (2026-10-08)
**Contexto:** O MVP precisa ser testável em qualquer PC, mas o produto exige codificação por hardware. Em notebooks híbridos, o MFT de uma GPU pode não aceitar o dispositivo D3D11 da outra.
**Decisão:** Enumerar MFTs H.264 (`MFTEnumEx`): hardware (NVIDIA/AMD/Intel, ordem do sistema) e depois os síncronos de software. Tentar cada candidato até um aceitar o `IMFDXGIDeviceManager` do dispositivo da captura e os tipos de mídia. Configuração: baixa latência, CBR, GOP de 2 s, 0 B-frames, perfil High (Main como alternativa), keyframe sob demanda via `CODECAPI_AVEncVideoForceKeyFrame`. Encoders assíncronos são tratados pelo modelo de eventos (`METransformNeedInput`/`HaveOutput`) sem bloquear. O encoder de software, se não aceitar Direct3D, recebe NV12 copiado para memória — **somente** nesse fallback, com aviso explícito ao usuário.
**Consequências:** Funciona em PCs sem encoder de hardware, com custo de CPU visível. NVENC/AMF/QSV via SDKs próprios ficam para a Etapa 6, se benchmarks justificarem.

## ADR-021 — Relógio único (QPC) para vídeo e áudio
**Status:** Aceita (2026-10-08)
**Contexto:** O WGC informa `SystemRelativeTime` em unidades de 100 ns derivadas do QPC; o WASAPI informa posições em ticks brutos de QPC.
**Decisão:** A sessão define a origem (`QueryPerformanceCounter`) **antes** de iniciar a captura. Vídeo: `pts_us = (SystemRelativeTime − origem_hns) / 10`. Áudio: o `vellcast-audio` recebe a origem em ticks brutos e converte para µs. Os dois chegam ao relay no mesmo eixo de tempo.
**Consequências:** Sincronização A/V depende só de os dois lados respeitarem a mesma origem; não há relógio de parede no caminho de mídia.

## ADR-022 — Fila "o mais recente vence" e controle de ritmo no callback da captura
**Status:** Aceita (2026-10-08)
**Contexto:** Monitores de 144 Hz entregam mais quadros que a taxa-alvo, e um encoder ocupado não pode acumular atraso.
**Decisão:** O callback do WGC aplica um controle de ritmo em grade (média exata na taxa-alvo, sem rajadas após inatividade) e coloca o quadro em uma fila de capacidade 1: um quadro novo substitui o pendente, que volta imediatamente ao pool. O encoder assíncrono só recebe quadro quando pede entrada; caso contrário o quadro é descartado e contabilizado.
**Consequências:** Memória e latência limitadas por construção. Sob carga, a transmissão perde quadros em vez de atrasar.

## ADR-026 — Player web servido pelo relay como cliente do MVP (S-011)
**Status:** Aceita (2026-10-08)
**Contexto:** Com o Discord adiado (ADR-013), o MVP precisa de um jeito de assistir sem instalar nada.
**Decisão:** O relay serve o build de `apps/discord-activity` em `/` e `/watch` (`VELLCAST_STATIC_DIR`, padrão: pasta `player` ao lado do executável). O mesmo player tem dois modos: Activity/Etapa 0 e relay (`/watch`). A API e `/ws` têm prioridade sobre os arquivos; o HTML sai com CSP restrita à própria origem, `nosniff` e `no-referrer`.
**Consequências:** Um único binário de servidor atende transmissor, espectadores e página; nenhum serviço web extra. A futura Activity reaproveita o mesmo player e protocolo.

## ADR-027 — Convite no fragmento da URL, trocado por token de espectador
**Status:** Aceita (2026-10-08)
**Decisão:** O link é `<relay>/watch#room=<id>&invite=<código>`. O fragmento nunca vai ao servidor nem a logs de proxy. O player troca o código (uso limitado) por um token de espectador via `POST /api/rooms/{id}/join`, guarda-o só na aba (`sessionStorage`), remove o código da barra de endereços (`history.replaceState`) e envia o token como primeira mensagem `Auth` do WebSocket. O desktop gera convites de até 10 usos válidos por 24 h.
**Consequências:** Recarregar a página reaproveita o token; uma nova aba ou outro aparelho consome mais um uso do convite.

## ADR-028 — HTTPS obrigatório para espectadores fora do PC; endereço público opcional
**Status:** Aceita (2026-10-08)
**Contexto:** O WebCodecs só existe em contexto seguro (HTTPS ou `localhost`). Um link `http://192.168.x.x` abre a página mas não decodifica vídeo.
**Decisão:** O desktop tem dois endereços: o do servidor (para transmitir e gerenciar salas) e um **endereço público opcional** para os links (túnel ou proxy HTTPS). O app avisa quando o link gerado só funciona neste PC ou não usa HTTPS. A documentação indica túnel HTTPS para testes e VPS com TLS (Dokploy) para uso contínuo.
**Consequências:** Teste em LAN puro (sem TLS) não é suportado no MVP. TLS embutido no relay (certificado próprio) pode ser avaliado se o túnel/proxy se mostrar uma barreira.

## ADR-029 — Desktop: motor em thread própria e HTTP em threads curtas
**Status:** Aceita (2026-10-08)
**Decisão:** `vellcast-core` roda na thread `vellcast-engine` (a sessão é criada e destruída nela, sem exigir `Send`). Chamadas ao relay usam `ureq` (síncrono, rustls) em threads curtas. Tudo chega à interface por `invoke_from_event_loop`. O timer de métricas (1 Hz) só roda com a transmissão ativa, a janela visível e a tela Ao vivo aberta; a contagem de espectadores é consultada a cada 3 s nessas mesmas condições.
**Consequências:** Nenhuma operação de captura, encoder ou rede bloqueia a interface; sem polling com a janela oculta. Mudar a qualidade durante a transmissão exige parar e iniciar de novo (aceito no MVP).

## ADR-030 — Salas do desktop persistidas com o token de dono
**Status:** Aceita (2026-10-08)
**Decisão:** As salas criadas ficam em `rooms.json` no diretório de configuração do usuário (até 20, expiradas removidas ao carregar), incluindo o token de dono necessário para convites, tokens de transmissor e encerramento. `Debug` de salas e configurações omite tokens e chaves; nada disso vai para logs. Um token de transmissor novo é pedido ao relay a cada início de transmissão.
**Consequências:** Reiniciar o relay (estado em memória) invalida as salas salvas; o app detecta `room_not_found`, remove a sala e orienta o usuário. Persistência no relay (E3-T11) elimina esse caso.

## ADR-031 — Conversão BGRA→NV12 na CPU quando não há processador de vídeo D3D11
**Status:** Aceita (2026-10-08)
**Contexto:** O diagnóstico na CI Windows (runner sem GPU) mostrou que a enumeração de fontes e do encoder de software funciona, mas o pipeline falhava em `ID3D11VideoDevice indisponível`: adaptadores básicos (VMs, Área de Trabalho Remota, driver genérico) não oferecem `ID3D11VideoProcessor`.
**Decisão:** Quando o processador de vídeo não existe, o encoder de software recebe NV12 convertido na CPU (`crates/encoder/src/cpu_convert.rs`: BT.709 estúdio, letterbox, vizinho mais próximo). Encoders de hardware continuam exigindo o caminho de GPU, sem nenhuma cópia para a CPU.
**Consequências:** O MVP funciona em qualquer Windows 10 2004+, inclusive sem GPU, com custo maior de CPU (o app já avisa quando usa codificação por software). A CI passa a exercitar o encoder real do Windows.
