# Riscos técnicos — Vellcast

> Registro vivo dos maiores riscos, ordenados por severidade (probabilidade × impacto). Revisar ao fim de cada etapa do [ROADMAP](ROADMAP.md).
> Escala: **Alta / Média / Baixa**. "Gatilho" é o sinal observável que indica que o risco se materializou.

Última revisão: 2026-10-08.

## Resumo

| # | Risco | Prob. | Impacto | Etapa de validação |
|---|---|---|---|---|
| R-01 | WebCodecs `VideoDecoder` H.264 indisponível ou instável em clientes Discord | Média | Alto | 0 |
| R-02 | Throughput/latência do proxy do Discord insuficientes para 1080p60 (~8 Mbps) | Média | Alto | 0 |
| R-03 | `AudioDecoder` Opus indisponível na Activity | Média | Alto | 0 |
| R-04 | Head-of-line blocking do WebSocket (TCP) sob perda de pacotes | Alta | Médio | 0 / 6 |
| R-05 | Políticas do Discord e indisponibilidade de Premium Apps no Brasil | Média | Alto | 7 |
| R-06 | Licenciamento do Slint incompatível com o modelo Open Core | Média | Alto | 1 |
| R-07 | Variação de encoders por hardware (NVENC/AMF/QSV) e drivers | Alta | Médio | 2 |
| R-08 | Limitações do Windows Graphics Capture | Média | Médio | 2 |
| R-09 | Application Loopback (áudio por processo) indisponível ou incompleto | Média | Médio | 4 |
| R-10 | Custo de banda do relay oficial | Alta | Médio | 3 / 7 |
| R-11 | Interações com anti-cheat | Baixa | Alto | 2 / 6 |
| R-12 | Conflito de foco/input do gamepad com o jogo em segundo plano | Média | Médio | 1 |
| R-13 | Mantenedor único / capacidade da equipe | Alta | Médio | Contínuo |
| R-14 | Ambiente de desenvolvimento macOS sem acesso a hardware Windows | Alta | Médio | 1–6 |

---

## Detalhamento

### R-01 — WebCodecs H.264 nos clientes Discord
- **Descrição:** a Activity roda no Discord Desktop (Electron/Chromium), no navegador (Discord Web) e em WebViews móveis. O suporte a `VideoDecoder` com `avc1.*`, decodificação por hardware e comportamento em Annex B pode variar; WebViews iOS/Android podem não expor WebCodecs ou ter perfis limitados.
- **Probabilidade:** Média · **Impacto:** Alto (bloqueia M-012).
- **Mitigação:** detectar com `VideoDecoder.isConfigSupported` antes de configurar; usar Constrained Baseline/Main sem B-frames; testar Desktop, Web e mobile na Etapa 0; manter a escolha de perfil/nível parametrizável no protocolo; fallback a avaliar (por exemplo, MSE com fMP4) documentado em DECISIONS antes de qualquer implementação.
- **Gatilho:** `isConfigSupported` retorna `supported: false`, erros de decodificação ou FPS decodificado abaixo do recebido.

### R-02 — Capacidade do proxy do Discord
- **Descrição:** todo tráfego da Activity passa pelo proxy do Discord (`{clientId}.discordsays.com`), que atualmente suporta apenas WebSocket. Não há limites de throughput ou tamanho de mensagem documentados.
- **Probabilidade:** Média · **Impacto:** Alto (afeta M-008).
- **Mitigação:** medir na Etapa 0 com 720p30 e 1080p60 a ~8 Mbps; limitar mensagens a tamanhos moderados; registrar latência e jitter; manter bitrate configurável e, se necessário, perfil padrão inferior na Activity com 1080p60 disponível onde o ambiente suportar.
- **Gatilho:** bitrate recebido abaixo do enviado, latência crescente ou desconexões no proxy.

### R-03 — `AudioDecoder` Opus na Activity
- **Descrição:** o suporte a `AudioDecoder` com `opus` pode variar entre clientes, e a reprodução depende de `AudioContext` (política de autoplay).
- **Probabilidade:** Média · **Impacto:** Alto (afeta M-009 e o critério de áudio da Etapa 4).
- **Mitigação:** detecção de capacidades; botão explícito para iniciar áudio (gesto do usuário); pesquisar fallback (decoder Opus em WebAssembly) antes de considerar o áudio concluído.
- **Gatilho:** `isConfigSupported` falso ou `AudioContext` suspenso.

### R-04 — Head-of-line blocking no WebSocket
- **Descrição:** WebSocket roda sobre TCP; uma perda de pacote atrasa todos os dados subsequentes, aumentando latência sob redes instáveis.
- **Probabilidade:** Alta · **Impacto:** Médio.
- **Mitigação:** filas curtas e descarte de frames antigos no servidor; ressincronização no keyframe; medir latência com perda simulada; avaliar QUIC (S-005) para o trecho transmissor→relay, onde o Discord não impõe o transporte.
- **Gatilho:** latência p95 crescente ao longo da sessão ou acúmulo de fila.

### R-05 — Políticas do Discord e monetização
- **Descrição:** Premium Apps tem restrições de localidade, paridade de preços/ofertas e verificação do app; a disponibilidade para desenvolvedores no Brasil não pode ser presumida. Mudanças de política podem afetar Activities.
- **Probabilidade:** Média · **Impacto:** Alto (Etapa 7).
- **Mitigação:** pesquisa documentada antes da Etapa 7; billing separado do núcleo; produto gratuito e self-hosted funcionais sem monetização via Discord.
- **Gatilho:** inelegibilidade confirmada ou mudança de política.

### R-06 — Licenciamento do Slint
- **Descrição:** o Slint oferece licenças distintas (GPLv3, royalty-free com atribuição, comercial). Vincular sob GPLv3 não permite distribuir o produto como Apache-2.0/proprietário sem condições. A licença Apache-2.0 do código próprio é apenas **proposta**.
- **Probabilidade:** Média · **Impacto:** Alto (bloqueia distribuição do desktop).
- **Mitigação:** decisão registrada em LICENSING/DECISIONS antes da Etapa 1 (E1-T01); inventário de licenças de terceiros; não publicar binários antes da validação.
- **Gatilho:** termos da licença royalty-free incompatíveis com o uso pretendido.

### R-07 — Variação de encoders por hardware
- **Descrição:** MFTs de NVIDIA, AMD e Intel expõem recursos diferentes (modo baixa latência, reconfiguração dinâmica, controle de taxa, keyframe sob demanda); drivers antigos podem falhar.
- **Probabilidade:** Alta · **Impacto:** Médio.
- **Mitigação:** detecção de capacidades por encoder; testes em ao menos uma GPU de cada fabricante; integrações específicas (por exemplo, NVENC SDK) apenas se justificadas por benchmark.
- **Gatilho:** falhas de configuração, latência de encode alta ou keyframes ignorados.

### R-08 — Limitações do Windows Graphics Capture
- **Descrição:** borda amarela de captura (removível apenas em builds recentes com API específica), diferenças entre versões do Windows 10/11, jogos em fullscreen exclusivo, HDR e DPI misto.
- **Probabilidade:** Média · **Impacto:** Médio.
- **Mitigação:** alvo inicial Windows 11; detectar recursos por versão; fallback DXGI Desktop Duplication para monitores se necessário; documentar limitações.
- **Gatilho:** captura preta, borda indesejada ou falha em jogos de referência.

### R-09 — Application Loopback (áudio por processo)
- **Descrição:** a captura por processo via WASAPI depende da build do Windows e pode não capturar áudio de processos filhos; o fallback de sistema pode capturar a voz do Discord.
- **Probabilidade:** Média · **Impacto:** Médio.
- **Mitigação:** detecção de suporte; incluir árvore de processos; aviso claro no fallback; testes com Discord aberto.
- **Gatilho:** silêncio na captura por processo ou eco da voz do Discord.

### R-10 — Custo de banda do relay
- **Descrição:** 1080p60 a ~8 Mbps por espectador gera ~3,6 GB/h por espectador; o plano Free em 1080p60 torna a banda o principal custo do serviço oficial.
- **Probabilidade:** Alta · **Impacto:** Médio.
- **Mitigação:** cotas definidas após testes de capacidade e custo; métricas de banda por sala; self-hosting como alternativa; não prometer tráfego ilimitado.
- **Gatilho:** custo por usuário ativo acima do orçamento.

### R-11 — Interações com anti-cheat
- **Descrição:** hooks globais, injeção ou leitura de memória de jogos podem ser detectados por anti-cheat.
- **Probabilidade:** Baixa (por design) · **Impacto:** Alto.
- **Mitigação:** apenas APIs públicas do Windows (WGC, WASAPI); nenhuma injeção (W-011); atalhos globais desativados por padrão; testes com jogos que usam anti-cheat populares.
- **Gatilho:** relatos de bloqueio ou alertas de anti-cheat.

### R-12 — Gamepad em segundo plano
- **Descrição:** ler o gamepad enquanto o jogo está em foco pode disparar ações no Vellcast (ou o contrário).
- **Probabilidade:** Média · **Impacto:** Médio.
- **Mitigação:** processar input do gamepad apenas com a janela do Vellcast em foco; atalhos globais opcionais e desativados por padrão.
- **Gatilho:** ações não intencionais durante o jogo.

### R-13 — Mantenedor único
- **Descrição:** escopo amplo (Rust nativo, GPU, áudio, servidor, frontend, billing) com equipe reduzida.
- **Probabilidade:** Alta · **Impacto:** Médio.
- **Mitigação:** MoSCoW estrito; etapas com critérios de aceite claros; documentação contínua; CI desde o início.
- **Gatilho:** etapas acumulando atraso ou itens Could Have iniciados antes dos Must Have.

### R-14 — Desenvolvimento em macOS
- **Descrição:** o ambiente atual é macOS; captura, encoder, WASAPI e gamepad no Windows não podem ser executados localmente.
- **Probabilidade:** Alta · **Impacto:** Médio.
- **Mitigação:** CI Windows para compilação e testes sem hardware; máquina Windows de referência para testes reais; tarefas de hardware marcadas `Pending Validation` até execução real.
- **Gatilho:** tarefas de hardware acumuladas sem validação.
