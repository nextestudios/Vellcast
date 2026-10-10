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
**Status:** Rejeitada (2026-10-09) — substituída pela ADR-058 (software proprietário)
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

## ADR-032 — Adaptação de qualidade por controlador AIMD com histerese
**Status:** Aceita (2026-10-08)
**Contexto:** S-001 pede ajuste de bitrate à rede; a medição em produção (DEPLOYMENT §6.10) mostrou rotas que sustentam só 1–2 Mbps por conexão.
**Decisão:** Controlador puro em `crates/vellcast-core/src/adapt.rs`, chamado uma vez por segundo pelo pipeline. Rede congestionada (descartes no transporte, fila de envio > ~0,5 s do bitrate atual ou RTT > 3× o mínimo observado e +150 ms) → bitrate ×0,7, no máximo uma redução a cada 2 s. Recuperação aditiva (+5% do máximo) só após 5 s estáveis; FPS só volta após 15 s. Limites: nunca acima da qualidade escolhida, piso de 500 kbps e 30 FPS. O bitrate é aplicado em tempo real com `CODECAPI_AVEncCommonMeanBitRate`; o FPS, descartando quadros antes do encoder.
**Consequências:** Testado com sinais sintéticos (11 testes). Encoders que recusam mudar o bitrate em tempo real ficam só com o ajuste de FPS (registrado em log); o comportamento por GPU precisa ser medido no Windows real.

## ADR-033 — Modo competitivo: bitrate → FPS → recomendação de resolução
**Status:** Aceita (2026-10-08)
**Contexto:** S-002 pede preservar o desempenho do jogo, reduzindo a carga da transmissão quando necessário, sem métricas invasivas do jogo.
**Decisão:** O perfil Competitivo ativa `Adaptation::Competitive`: a sobrecarga do encoder (quadros recusados ou tempo de codificação > 85% do intervalo entre quadros) reage já na primeira janela, primeiro reduzindo o bitrate (×0,8 até 60% do máximo), depois o FPS (60 → 45 → 30 → 24). A resolução não muda durante a transmissão (exigiria reiniciar o encoder e o decoder do espectador): no FPS mínimo, o app recomenda transmitir de novo com uma resolução menor. A prioridade do processo nunca é aumentada.
**Consequências:** Reação rápida à carga com recuperação mais lenta (10 s para bitrate, 30 s para FPS). Sinais vêm só do próprio pipeline; nada é lido do processo do jogo.

## ADR-034 — Métricas avançadas do próprio processo sem dependência extra
**Status:** Aceita (2026-10-08)
**Contexto:** S-007 pede CPU e RAM do app junto das métricas da transmissão.
**Decisão:** `apps/desktop/src/procstats.rs` lê `GetProcessTimes` e `GetProcessMemoryInfo` (memória privada) no Windows, via a crate `windows` já usada no projeto; fora do Windows os valores aparecem como "—". A amostragem acontece só quando a tela Ao vivo recebe métricas (1 Hz, tela visível).
**Consequências:** Sem `sysinfo` nem threads extras. O valor de CPU é relativo à máquina inteira (todos os núcleos).

## ADR-050 — OAuth2 do Discord trocado no relay; sessão Vellcast própria
**Status:** Aceita (2026-10-08)
**Contexto:** A Activity precisa identificar o usuário (M-014) sem expor o client secret e sem confiar em identidade enviada pelo frontend.
**Decisão:** O relay recebe o `code` do `authorize`, troca por access token em `oauth2/token`, confirma o usuário em `users/@me` e emite uma sessão HMAC de 2 h (`room = "discord-user:<id>"`, que nunca colide com IDs de sala). O cliente HTTP é o `ureq` 3 (rustls), já presente no workspace e executado em `spawn_blocking`, em vez de adicionar `reqwest`. Recurso desligado sem `VELLCAST_DISCORD_CLIENT_ID/SECRET`.
**Consequências:** Uma única peça de backend (o relay) atende o MVP e o self-hosting; a Cloud API pode assumir as contas depois sem mudar a Activity.

## ADR-051 — Pareamento por código curto em vez de login no desktop
**Status:** Aceita (2026-10-08)
**Contexto:** M-015 pede vincular o app Windows a uma sessão da Activity sem compartilhar credenciais do Discord.
**Decisão:** O dono da sala gera no desktop um código de 8 caracteres (alfabeto sem ambiguidades, 5 min, uso único, até 3 ativos por sala). Digitado na Activity, o código vincula a instância (`instanceId`) à sala até ela expirar; todos os participantes da instância passam a assistir. Estado em memória, como as salas. Checagem opcional de participação na instância com token de bot.
**Consequências:** O desktop não precisa de OAuth nem de navegador embutido. Sem token de bot, quem tiver uma sessão e o `instanceId` vê as salas da instância (risco aceito no MVP; ativar o bot fecha a brecha).

## ADR-045 — Instalador Windows com Inno Setup, por usuário
**Status:** Aceita (2026-10-08)
**Contexto:** S-016 pede instalação, atualização e desinstalação simples. Opções: Inno Setup 6, WiX v4 (MSI) e MSIX.
**Decisão:** Inno Setup 6 (`infrastructure/installer/vellcast.iss`), instalação por usuário sem administrador (opcional para todos), atalho no Menu Iniciar, início com o Windows **desligado por padrão**, `CloseApplications` para atualizar com o app aberto e desinstalação que pergunta antes de apagar configurações. Compilado e testado em modo silencioso na CI (`release.yml` e `smoke.yml`, via `scripts/windows/`).
**Consequências:** Um único `.exe` simples de assinar e atualizar. MSI (implantação corporativa via GPO) e MSIX (Microsoft Store) ficam para depois, se houver demanda.

## ADR-046 — Atualizações assinadas com Ed25519, iniciadas pelo usuário
**Status:** Aceita (2026-10-08)
**Contexto:** S-012 pede atualizações seguras sem serviços pesados; o spec proíbe processamento oculto.
**Decisão:** Manifesto JSON (`version`, `url` HTTPS, `sha256`, `notes`) com assinatura Ed25519 destacada (`.sig`, base64), gerados por `tools/release-sign` no workflow de release. O app (`apps/desktop/src/updater.rs`, `ed25519-dalek`) verifica a assinatura com a chave pública embutida **antes** de interpretar o manifesto, compara a versão, baixa o instalador para a pasta temporária, confere o SHA-256 e só abre o instalador depois do clique do usuário. Sem verificação automática em segundo plano. URL padrão: ativo do último release do GitHub; `VELLCAST_UPDATE_URL` permite outro servidor.
**Consequências:** Comprometer o host do download não basta para distribuir binário malicioso. A chave privada fica só no segredo `UPDATE_SIGNING_KEY`; perdê-la exige publicar uma versão com nova chave pública por outro canal. Enquanto o repositório for privado, a URL padrão não é acessível sem autenticação (usar `VELLCAST_UPDATE_URL` ou tornar o repositório público).

## ADR-047 — Assinatura de código opcional na CI
**Status:** Aceita (2026-10-08) — certificado pendente
**Decisão:** Os passos de `signtool` (binários e instalador, SHA-256 com carimbo de tempo) só rodam quando os segredos `WINDOWS_CERT_PFX_BASE64` e `WINDOWS_CERT_PASSWORD` existem; sem eles, a CI emite um aviso e segue.
**Consequências:** Builds sem certificado continuam funcionando para testes; o SmartScreen avisa até haver um certificado (OV ou EV) comprado pela Next Estúdios.

## ADR-048 — Inventário de licenças automatizado (cargo-deny e cargo-about)
**Status:** Aceita (2026-10-08)
**Decisão:** `deny.toml` define as licenças aceitas (permissivas + CDLA-Permissive-2.0 dos certificados raiz) e trata o Slint por exceção (opção Royalty-free 2.0, ADR-006); o job "Licenças e segurança" da CI roda `cargo deny check licenses advisories bans sources` e verifica as licenças npm do player. O pacote e o instalador trazem `THIRD-PARTY-NOTICES.txt` gerado por `cargo-about` + `license-checker`.
**Consequências:** Dependência com licença nova ou incompatível quebra a CI até ser avaliada; vulnerabilidades conhecidas (RustSec) também.

## ADR-049 — Higiene do repositório para abertura do código
**Status:** Aceita (2026-10-08)
**Decisão:** `SECURITY.md` com GitHub Private Vulnerability Reporting (sem e-mail pessoal), `CODE_OF_CONDUCT.md` (Contributor Covenant 2.1), modelos de issue e PR, varredura de segredos com gitleaks na CI (falsos positivos em `.gitleaksignore`) e revisão de segurança registrada em `docs/security/`.
**Consequências:** O repositório fica pronto para abertura assim que o responsável decidir licença, separação da parte comercial e marca (OC-T01, OC-T03, OC-T04, OC-T09).

## ADR-040 — Cloud API em Axum + SQLite (rusqlite), esquema portável para PostgreSQL
**Status:** Aceita (2026-10-08)
**Contexto:** A Etapa 7 precisa de contas, planos, assinaturas, entitlements e cotas. A spec pede solução simples para começar e avaliar PostgreSQL para a infraestrutura comercial, sem Redis sem necessidade demonstrada.
**Decisão:** `services/cloud-api` em Axum com SQLite embutido via `rusqlite` 0.40 (feature `bundled`, MIT; SQLite em domínio público). O acesso é síncrono atrás de um `Mutex` (operações curtas); o hash de senha roda em `spawn_blocking`. Migrações por `PRAGMA user_version`. O esquema usa só TEXT/INTEGER, chaves e índices simples, sem recursos exclusivos do SQLite além de `ON CONFLICT`, que também existe no PostgreSQL. `sqlx` foi descartado por exigir mais dependências e consultas assíncronas que não trazem ganho neste volume.
**Consequências:** Um arquivo de banco por instância (volume `/data`), sem serviço externo. Escalar para várias instâncias exige migrar para PostgreSQL; a camada `Db` concentra todo o SQL para facilitar a troca.

## ADR-041 — Senhas Argon2id e sessões opacas
**Status:** Aceita (2026-10-08)
**Decisão:** Senhas com Argon2id (m = 19 MiB, t = 2, p = 1; recomendação OWASP) via `argon2` 0.6 (`hash_password_into`, formato próprio versionado), mínimo de 10 caracteres. Sessões com token aleatório de 32 bytes; o banco guarda só o SHA-256. Entrega em cookie `HttpOnly; SameSite=Lax; Secure` ou `Authorization: Bearer` para o app desktop. Login e cadastro limitados por IP; contas inexistentes passam pela mesma verificação de custo para não revelar e-mails pelo tempo de resposta. O painel exige token CSRF (HMAC do hash da sessão) em todo formulário.
**Consequências:** Sem confirmação de e-mail nem recuperação de senha nesta versão (exigem envio de e-mail; ficam para antes da cobrança real).

## ADR-042 — Entitlements como tokens HMAC curtos verificados pelo relay hospedado
**Status:** Aceita (2026-10-08)
**Contexto:** O relay oficial precisa aplicar limites de plano sem consultar a cloud-api a cada pedido, e o relay self-hosted não pode depender dela (W-013).
**Decisão:** A cloud-api emite tokens `ve1.<payload>.<hmac>` de 10 minutos com plano, espectadores por sala, salas por conta e resolução/FPS máximos (crate `vellcast-entitlement`). O relay só os exige quando `VELLCAST_CLOUD_ENTITLEMENT_KEY` está definida e então limita espectadores por sala e salas ativas por conta. Sem a chave, nada muda. A resolução nunca é limitada abaixo de 1080p60 (W-014).
**Consequências:** Revogação imediata não existe (vale até expirar, no máximo 10 min). A contagem de salas por conta fica em memória no relay, consistente com o armazenamento atual das salas.

## ADR-043 — Assinaturas vendidas fora do Discord até haver elegibilidade
**Status:** Aceita (2026-10-08)
**Contexto:** Em 2026-10-08, a documentação oficial do Discord restringe Premium Apps a desenvolvedores dos EUA, da UE e do Reino Unido; o Brasil não é elegível (MONETIZATION §6).
**Decisão:** A cobrança acontece fora do Discord, por provedor de pagamento integrado à cloud-api (Stripe implementado em modo de teste; Mercado Pago avaliado para BRL/Pix). A Activity não exibe preços nem links de compra até a revisão dos Monetization Terms e da Developer Policy vigentes.
**Consequências:** Reavaliar se o Discord ampliar as regiões; a arquitetura de entitlements já é independente do provedor.

## ADR-044 — Webhooks de cobrança verificados, idempotentes e com máquina de estados simples
**Status:** Aceita (2026-10-08)
**Decisão:** O Vellcast nunca processa cartões. O webhook verifica `Stripe-Signature` (HMAC-SHA256 de `t.corpo`, tolerância de 5 minutos contra replay), registra o ID do evento para idempotência e aplica `customer.subscription.created/updated/deleted`. Estados internos: `active` (inclui `trialing`), `past_due` (mantém o plano até o fim do período pago) e `canceled`. Uma assinatura cancelada não é reativada por evento atrasado de período anterior. Eventos para contas inexistentes são aceitos e ignorados (evita reenvio infinito do provedor).
**Consequências:** Ativação real exige a conta do responsável no provedor e o checkout gravando `account_id` e `plan` nos metadados da assinatura.

## ADR-036 — Escrita em lote para espectadores e transmissor; benchmarks em `crates/perf`
**Status:** Aceita (2026-10-08)
**Contexto:** E6-T01/E6-T02 pediam benchmarks reproduzíveis e redução de alocações e cópias no relay, com números antes/depois.
**Decisão:** (1) `crates/perf` (não distribuído) concentra os benchmarks: `fanout_bench` mede o relay em processo com um alocador contador restrito às threads do relay; `quic_eval` compara transportes. O crate permite `unsafe` só para o alocador contador. (2) Cada espectador drena as mensagens pendentes (até 64) com `feed` e faz um único `flush`; o snapshot inicial (configuração + GOP em cache) também vira uma única escrita; o transmissor (`crates/transport`) drena sua fila do mesmo jeito. Configurável por `VELLCAST_BATCH_WRITES=0` para comparação.
**Consequências:** Medido: o relay continua sem alocação por espectador (0,03 alocação por mensagem entregue, só o buffer de entrada) e o p99 local caiu de 2,3–3,0 ms para ~1,7 ms; CPU dentro do ruído. A primeira versão do lote alocava um `Vec` por mensagem; o benchmark detectou e a versão final reaproveita o vetor. Resultados em `PERFORMANCE.md` §3.5.

## ADR-037 — QUIC entre desktop e relay: adiado; se adotado, Quinn com BBR e um stream por quadro
**Status:** Aceita (2026-10-08)
**Contexto:** S-005 pedia avaliar QUIC (Quinn) contra o WebSocket/TCP atual. A Activity do Discord só aceita WebSocket (DISCORD_INTEGRATION.md), então a avaliação vale apenas para o trecho desktop → relay.
**Medições** (`PERFORMANCE.md` §3.6, loopback com perda/atraso simulados): sem perda, QUIC e TCP empatam na latência mediana (~24 ms com 20 ms de atraso de ida). Com Cubic (padrão do Quinn), o QUIC desaba com 1–3% de perda (315/600 e 81/600 quadros entregues), como prevê a fórmula de Mathis para controle de congestionamento baseado em perda (≈2,9 Mbps sustentáveis com 1% de perda e RTT de 40 ms, abaixo dos 8 Mbps). Com BBR, o QUIC entregou 600/600 em todos os cenários, p99 de 83 ms com 1% de perda e recuperação de 27 ms após um apagão de 300 ms (TCP modelado: 203 ms; QUIC Cubic: 781 ms). O QUIC custou ~6× mais CPU no processo (10–12% contra 1,3–1,8%), por criptografia e processamento por pacote em espaço de usuário. O modelo de TCP não reduz a janela após perdas, então um TCP real com Cubic sofreria o mesmo limite do QUIC Cubic.
**Decisão:** Manter WebSocket/TCP como único transporte no MVP. Não integrar QUIC agora: o ganho só aparece em redes com perda, custa CPU no PC do jogador (prioridade 1 do projeto), exige UDP liberado no servidor (Traefik/Dokploy não encaminham QUIC para a aplicação sem configuração extra) e precisaria de fallback para TCP de qualquer forma. Se for adotado depois: Quinn com **BBR**, um stream unidirecional por quadro com prioridade FIFO e fallback automático para WebSocket.
**Ação de baixo custo derivada:** a rota Europa → Brasil medida em produção limitou uma conexão TCP a 1,2–2,2 Mbps (DEPLOYMENT.md §6.10). Habilitar **BBR no kernel do servidor do relay** (`net.ipv4.tcp_congestion_control=bbr` com `net.core.default_qdisc=fq`) aplica o mesmo princípio ao TCP existente, sem mudar o protocolo; deve ser medido no VPS antes e depois.
**Reavaliar quando:** houver medições em redes reais com perda (Wi-Fi/4G do transmissor) mostrando quadros atrasados que o BBR no servidor não resolva.


## ADR-057 — Fallback DXGI Desktop Duplication não é necessário no MVP
**Status:** Aceita (2026-10-09)
**Contexto:** E2-T03 previa implementar DXGI Desktop Duplication apenas se o Windows Graphics Capture falhasse nos testes reais.
**Evidência:** no Windows real (workflow `windows-e2e`), o WGC capturou monitor e janela, codificou H.264 e transmitiu pelo relay a espectadores (ver PERFORMANCE §3.3).
**Decisão:** não implementar o fallback agora. Reabrir se testes com jogos em tela cheia exclusiva ou drivers específicos mostrarem falhas do WGC.
**Consequências:** menos código nativo e superfície de manutenção; WGC exige Windows 10 2004+, já requisito do produto.

## ADR-055 — Salas persistentes em SQLite opcional no relay
**Status:** Aceita (2026-10-09)
**Contexto:** Com o estado só em memória, cada reinício ou redeploy do relay apagava salas, convites e links (E3-T11). O trait `RoomStore` já separava metadados do caminho de mídia.
**Decisão:** `SqliteStore` (`rusqlite` 0.40.2 com SQLite embutido, a mesma versão da cloud-api) ativado por `VELLCAST_DB_PATH`; WAL, `synchronous=NORMAL`, chaves estrangeiras e migrações por `PRAGMA user_version`. Guarda só salas, convites (hash) e revogações. Na inicialização, o relay recria o estado ao vivo de cada sala não expirada. Sem a variável, o comportamento continua em memória.
**Consequências:** Links e tokens sobrevivem a redeploys quando o volume `/data` é montado. Continua uma réplica só (um escritor no SQLite); mídia nunca vai para disco. Para a Cloud com várias réplicas, uma implementação PostgreSQL do mesmo trait.

## ADR-056 — Revogação de tokens por nonce e rotação do segredo com segredo anterior
**Status:** Aceita (2026-10-09)
**Contexto:** Tokens são credenciais *bearer* válidas até expirar; a única revogação era encerrar a sala ou trocar o segredo (derrubando tudo) (E3-T12).
**Decisão:** (a) O dono revoga um token específico (o relay verifica o token informado e guarda o `nonce` até a expiração) ou todos os tokens de espectador emitidos antes de um instante (novo campo `iat` nos tokens; tokens antigos sem `iat` contam como emitidos no início). A revogação é checada na API e no `Auth` do WebSocket, e um sinal por sala faz as conexões ativas reavaliarem seus tokens. (b) `VELLCAST_RELAY_SECRET_PREVIOUS` é aceito só na verificação, permitindo rotação sem queda.
**Consequências:** Vazamentos são contidos sem encerrar a sala. Cada verificação consulta o store (leitura curta). A revogação em massa usa granularidade de segundos: tokens emitidos no mesmo segundo da revogação também ficam inválidos.

## ADR-058 — Vellcast como software proprietário; repositório privado; parte comercial separada
**Status:** Aceita (2026-10-09, decisão do responsável)
**Contexto:** O Marco OC (Open Core) previa abrir o núcleo sob Apache-2.0 (ADR-007, proposta). O responsável decidiu não abrir o código.
**Decisão:** Todo o código do Vellcast é proprietário (todos os direitos reservados); o repositório `nextestudios/Vellcast` permanece privado; a parte comercial (`cloud-api`) vai para o repositório privado `nextestudios/vellcast-cloud`. O formato do token de entitlement continua no núcleo (`crates/entitlement`), usado pelo relay.
**Consequências:** OC-T01 e OC-T09 resolvidos por decisão (licença proprietária; repositório privado). O inventário de licenças de terceiros, a varredura de segredos e a política de segurança continuam valendo. A licença Royalty-free do Slint permite o app proprietário com atribuição.

## ADR-059 — Gitflow e workflows do GitHub no padrão do ControlFS
**Status:** Aceita (2026-10-10, pedido do responsável)
**Contexto:** A CI rodava a cada push na `main` (incluindo o E2E de Windows e o pacote), as releases eram rascunhos criados por qualquer tag e a versão do `Cargo.toml` só era ajustada no build da tag. O responsável pediu o mesmo fluxo do outro projeto da equipe, o ControlFS.
**Decisão:** Gitflow com a regra pré-1.0 do ControlFS (sem `develop`; PRs de `feature/*`/`fix/*`/`hotfix/*` direto na `main`). CI só em pull requests para `main`/`develop`, com `paths-ignore` de docs e cancelamento de execuções antigas. `release.yml` publica por tag `vX.Y.Z` na `main` (guarda Gitflow, versão do `Cargo.toml` igual à tag, notas da seção do `CHANGELOG.md`, testes completos, smoke do instalador, manifesto assinado obrigatório, release publicada como Latest). `smoke.yml`, `windows-e2e.yml` e `diag-audio.yml` sob demanda. Dependabot mensal agrupado (cargo, npm, actions). CodeQL em push na `main` e semanal, pulado enquanto o repositório for privado. O empacotamento sai dos workflows para `scripts/windows/` (como o `build/` do ControlFS). Diferenças conscientes: changelog só em pt-BR (o projeto é documentado em pt-BR) e sem portátil nem manifesto do WinGet (o repositório é privado e os downloads exigem autenticação).
**Consequências:** PRs não geram pacotes nem rodam o E2E pesado; quem altera captura, encoder, áudio, transporte ou relay roda o **Windows E2E** antes da release. Releases deixam de ser rascunhos e exigem o PR `chore(release)`.
