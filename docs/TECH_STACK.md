# Stack tecnológica

Cada dependência é avaliada por manutenção, licença, segurança, tamanho, impacto no desempenho, suporte a Windows e compatibilidade com Rust estável. Dependências pesadas só entram com justificativa registrada em [DECISIONS.md](DECISIONS.md).

## 1. Visão por componente

| Camada | Tecnologia | Justificativa | Status |
|---|---|---|---|
| Linguagem | Rust estável (edition 2024) | Desempenho previsível, sem GC, segurança de memória | Aceita |
| UI desktop | Slint | Nativo, leve, renderização sob demanda, sem navegador | Aceita (licença pendente, ADR-006) |
| Controles | GilRs + XInput/Windows.Gaming.Input quando necessário | GilRs cobre a maioria; APIs nativas para casos específicos | A validar na ETAPA 1 |
| Captura | Windows Graphics Capture; DXGI Desktop Duplication como alternativa | API oficial, captura por janela, frames em textura D3D11 | Aceita |
| GPU | Direct3D 11 | Interoperável com WGC e Media Foundation | Aceita |
| Encoder | Media Foundation (MFT de hardware); NVENC/AMF/QSV direto se benchmarks justificarem | Uma API cobre os três fabricantes | Candidata |
| Codec de vídeo | H.264 | Suporte universal em WebCodecs e decoders de hardware | Aceita |
| Áudio | WASAPI (Application Loopback por processo) | Captura nativa por processo no Windows 10 2004+ | Candidata |
| Codec de áudio | Opus 48 kHz, 20 ms | Baixa latência, suportado em WebCodecs `AudioDecoder` | A validar na ETAPA 0 |
| Transporte inicial | WebSocket binário sobre TLS | Único transporte suportado pelo proxy de Activities | Aceita |
| Transporte nativo | QUIC (Quinn) | Evita head-of-line blocking; avaliar contra WS (S-005) | Avaliação |
| Backend | Tokio + Axum | Ecossistema maduro, WebSocket nativo no Axum | Aceita |
| Activity | React + TypeScript + Vite | Leve, build estático, ecossistema do Embedded App SDK | Aceita |
| SDK Discord | `@discord/embedded-app-sdk` | Oficial | Aceita |
| Decodificação web | WebCodecs (`VideoDecoder`, `AudioDecoder`) | Decodificação por hardware sem MSE/WebRTC | A validar na ETAPA 0 |
| Banco | SQLite (self-hosting); PostgreSQL (Cloud, avaliar) | Zero operação para self-hosters | Proposta (não usado ainda) |
| Containers | Docker, imagem mínima | Deploy reproduzível | Aceita |
| Deploy | VPS Linux + Dokploy + proxy TLS | Infra própria de baixo custo | Aceita |
| Observabilidade | `tracing` (logs estruturados), endpoint de métricas, `/health` | Sem serviços externos obrigatórios | Aceita |

## 2. Dependências atuais (ETAPA 0)

### Rust (workspace)
| Crate | Uso | Licença | Observação |
|---|---|---|---|
| `tokio` | Runtime assíncrono | MIT | Features mínimas |
| `axum` (feature `ws`) | HTTP + WebSocket | MIT | |
| `tower-http` | Servir os estáticos da Activity | MIT | Feature `fs` |
| `bytes` | Payloads com contagem de referência (fan-out sem cópia) | MIT | |
| `serde`, `serde_json` | Mensagens Init em JSON | MIT/Apache-2.0 | |
| `tracing`, `tracing-subscriber` | Logs estruturados | MIT | |
| `anyhow` / `thiserror` | Erros | MIT/Apache-2.0 | `thiserror` em bibliotecas, `anyhow` em binários |
| `futures-util` | Split do WebSocket | MIT/Apache-2.0 | |

`vellcast-protocol` não tem dependências de I/O, o que mantém o crate portável para o desktop, o relay e as ferramentas.

### Activity (npm)
| Pacote | Uso |
|---|---|
| `@discord/embedded-app-sdk` | Handshake com o cliente Discord |
| `react`, `react-dom` | UI do player |
| `vite`, `@vitejs/plugin-react`, `typescript` | Build |
| `@types/dom-webcodecs` | Tipagem do WebCodecs, se a lib DOM do TS não cobrir |

As versões ficam fixadas no `package.json`/`Cargo.lock`. Sempre se prefere uma versão publicada há pelo menos duas semanas.

### Ferramentas de desenvolvimento
- `ffmpeg` (com libx264 e libopus), usado **somente** para gerar a mídia sintética da ETAPA 0. Não é dependência de runtime.

## 3. Rejeitados ou adiados

| Opção | Motivo |
|---|---|
| Electron / navegador embutido no transmissor | W-001/W-002: custo de memória e CPU incompatível com as metas |
| WebRTC na Activity | Não suportado pelo proxy do Discord (W-005) |
| WebTransport | Ainda não suportado nas Activities; reavaliar quando o Discord habilitar |
| Redis | Sem necessidade demonstrada; o estado das salas fica em memória no relay |
| Microsserviços | O relay e a cloud-api bastam; separar só sob demanda medida |
| Captura por injeção/hook | W-011/W-012: risco de anti-cheat e privacidade |
| Codecs H.265/AV1 | Could Have (C-003/C-004); o suporte no WebCodecs é desigual |

## 4. Plataformas

| Componente | Alvo | Ambiente de desenvolvimento atual |
|---|---|---|
| Desktop | Windows 11 x64 | macOS: só compila crates portáveis; captura/GPU/áudio exigem máquina Windows real |
| Relay | Linux x86_64/arm64 (Docker) | Executável localmente no macOS |
| Activity | Discord Desktop, Web e móvel (a validar) | Testável no navegador local; validação real exige app Discord + túnel HTTPS |

O CI no Windows valida **compilação e testes unitários**. Ele não substitui testes reais de captura e GPU.
