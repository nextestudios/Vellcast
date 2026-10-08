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
