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

