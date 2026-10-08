# Licenças — Vellcast

> Este documento não é aconselhamento jurídico. Decisões de licença devem ser confirmadas pelo responsável da Next Estúdios antes da publicação.

## 1. Código próprio

**Proposta:** Apache-2.0 para o código aberto próprio (arquivo `LICENSE` adicionado na raiz). **Status: pendente de confirmação do proprietário.**

### Apache-2.0 × MIT

| Aspecto | Apache-2.0 | MIT |
|---|---|---|
| Permissiva (uso comercial, modificação, sublicença) | Sim | Sim |
| **Concessão explícita de patentes** | Sim — cada contribuidor concede licença de patentes sobre sua contribuição | Não explícita |
| **Retaliação de patentes** | Quem processar alegando violação de patente perde a licença | Não há |
| Arquivo **NOTICE** | Deve ser preservado em redistribuições, se existir | Não existe |
| Aviso de modificações | Arquivos modificados devem indicar a alteração | Não exige |
| Marca registrada | Explicitamente **não** concedida (cláusula 6) | Silenciosa |
| Compatibilidade com GPL | Compatível com GPLv3, **incompatível com GPLv2-only** | Compatível com ambas |
| Tamanho/simplicidade | Texto longo | Muito curto |

Motivo da proposta: o projeto lida com codecs, streaming e captura — áreas com histórico de patentes. A concessão explícita e a cláusula de marca protegem contribuidores, usuários e o nome "Vellcast". Alternativa comum no ecossistema Rust: dupla licença `MIT OR Apache-2.0`, que maximiza compatibilidade (inclusive GPLv2). Pode ser adotada se a compatibilidade com GPLv2 for desejada.

## 2. Slint — decidido: Royalty-free 2.0 (ADR-006)

O Slint oferece licenciamento múltiplo. Opções conhecidas (texto **deve ser verificado na versão vigente** em https://slint.dev antes da Etapa 1):

| Opção | Resumo | Impacto no Vellcast |
|---|---|---|
| **GPLv3** | Software que usa Slint e é distribuído deve ser distribuído sob termos GPLv3-compatíveis | O binário do desktop distribuído seria GPLv3 como um todo. O código próprio pode continuar Apache-2.0 (compatível com GPLv3), mas o **produto combinado** fica sob GPLv3. **Não** permite distribuir o desktop como proprietário. |
| **Royalty-free (2.0)** | Gratuita para aplicações desktop, mobile e web, com **obrigação de atribuição** (ex.: exibir "Made with Slint"/badge e link na aplicação ou na página de download, conforme o texto da licença) | Compatível com distribuir o desktop sob Apache-2.0 ou fechado, **desde que** se cumpra a atribuição e o tipo de aplicação seja coberto (historicamente **exclui sistemas embarcados**). Verificar se há outras condições (ex.: proibição de reexpor a API do Slint). |
| **Comercial** | Licença paga, sem atribuição obrigatória, com suporte | Útil se a atribuição for indesejada ou se o produto entrar em embarcados. |

**Decisão (2026-10-08):** o app desktop usa o Slint sob a **Slint Royalty-free Desktop, Mobile, and Web Applications License, versão 2.0**. Texto lido em https://slint.dev/terms-and-conditions na mesma data.

Condições vigentes e como o Vellcast as cumpre:

| Condição da licença | Como cumprimos |
|---|---|
| Cobre apps desktop, mobile e web; **exclui sistemas embarcados** | O Vellcast é um app desktop Windows. Qualquer porte para embarcados exige outra licença. |
| Atribuição: widget `AboutSlint` numa tela "Sobre" acessível pelo menu principal **ou** badge do Slint em página pública (de preferência a de download) | Faremos os dois: `AboutSlint` na tela 08 "Sobre" (acessível pela barra lateral) e o badge na página de download. |
| Não distribuir o Slint isoladamente | O Slint só é distribuído embutido no app. |
| Não expor as APIs do Slint pelo aplicativo | O app não expõe APIs de UI (o sistema de plugins, C-018, precisa respeitar isso). |
| Manter avisos de licença e copyright | Inventário de licenças de terceiros na tela "Sobre" e no instalador. |

O código próprio continua sob Apache-2.0; o binário distribuído combina esse código com o Slint sob a licença acima. Forks do código aberto que distribuírem o desktop precisam cumprir a mesma atribuição ou escolher outra licença do Slint.

Não presumir que vincular Slint sob GPLv3 permite distribuir o produto final inteiro como proprietário ou "só Apache-2.0".

## 3. Codecs e mídia

- **H.264:** coberto por patentes (pool Via LA, ex-MPEG LA). Usar os encoders/decoders de hardware fornecidos pelo sistema/driver (Media Foundation, NVENC/AMF/QSV) e o decoder do navegador (WebCodecs) — o licenciamento normalmente recai sobre o fabricante/plataforma, mas **confirmar antes do lançamento comercial**.
- **Opus:** licença BSD, livre de royalties. Para a biblioteca libopus, verificar o binding Rust escolhido na Etapa 4.
- **FFmpeg / libx264:** usado **apenas como ferramenta de desenvolvimento** para gerar mídia sintética de teste (`scripts/`), **não é distribuído** nem vinculado aos binários. Se algum dia for distribuído: libx264 é GPLv2+, o que tornaria o build do FFmpeg GPL e imporia obrigações GPL ao que for distribuído junto/vinculado. Evitar.
- Mídia de teste é gerada sinteticamente (padrões de teste e tons). Não incluir vídeos/músicas de terceiros.

## 4. Inventário de licenças de terceiros

Estado na Etapa 0. A coluna "Verificado" indica conferência no registro do pacote; a lista completa (incluindo transitivas) será gerada automaticamente antes de qualquer distribuição (`cargo deny` / `cargo about` para Rust, `license-checker` ou equivalente para npm).

### Rust (servidor de validação da Etapa 0, `crates/protocol`)

| Crate | Licença | Uso | Verificado |
|---|---|---|---|
| tokio | MIT | Runtime assíncrono | Pendente (cargo deny) |
| axum | MIT | HTTP/WebSocket | Pendente |
| tower-http | MIT | Arquivos estáticos | Pendente |
| tracing | MIT | Logs estruturados | Pendente |
| tracing-subscriber | MIT | Saída de logs | Pendente |
| serde / serde_json | MIT OR Apache-2.0 | Serialização | Pendente |
| bytes | MIT | Buffers sem cópia | Pendente |
| anyhow | MIT OR Apache-2.0 | Erros (binários) | Pendente |
| thiserror | MIT OR Apache-2.0 | Erros (bibliotecas) | Pendente |

### JavaScript (Activity da Etapa 0)

| Pacote | Licença | Uso | Verificado |
|---|---|---|---|
| @discord/embedded-app-sdk | MIT | Integração com Activity | Sim (npm) |
| react / react-dom | MIT | UI do player | Sim (npm, react) |
| vite | MIT | Build (dev, não distribuído no bundle) | Sim (npm) |
| typescript | Apache-2.0 | Build (dev) | Pendente |
| @vitejs/plugin-react | MIT | Build (dev) | Pendente |

### Rust (app desktop e `crates/controller`, Etapa 1)

Versões fixadas com `=` no `Cargo.toml`. Licenças conferidas no crates.io em 2026-10-08.

| Crate | Versão | Licença | Uso | Verificado |
|---|---|---|---|---|
| slint / slint-build | 1.18.1 | GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0 (usamos Royalty-free 2.0, ver §2) | UI desktop | Sim (crates.io) |
| gilrs | 0.11.2 | Apache-2.0 / MIT | Entrada de gamepad | Sim (crates.io) |
| tray-icon | 0.24.0 | MIT OR Apache-2.0 | Ícone na bandeja | Sim (crates.io) |
| muda (via tray-icon e Slint) | 0.19.3 | MIT OR Apache-2.0 | Menu da bandeja | Pendente (cargo deny) |
| directories | 6.0.0 | MIT OR Apache-2.0 | Pasta de configuração do sistema | Sim (crates.io) |
| serde / serde_json | — | MIT OR Apache-2.0 | Arquivo de configurações | Pendente (cargo deny) |
| ureq | 3.4.2 | MIT OR Apache-2.0 | Cliente HTTP da API de salas (rustls + ring; sem OpenSSL) | Sim (crates.io) |
| arboard | 3.6.1 | MIT OR Apache-2.0 | Copiar o link de convite (sem suporte a imagens) | Sim (crates.io) |

Dependências transitivas do Slint (winit, softbuffer, accesskit, fontique/skrifa etc.) entram no inventário automático (`cargo about`), ainda **pendente**.

### Rust (relay e `crates/transport`, Etapa 3)

Licenças lidas do `cargo metadata` em 2026-10-08.

| Crate | Versão | Licença | Uso | Observação |
|---|---|---|---|---|
| hmac / sha2 | 0.13.0 / 0.11.0 | MIT OR Apache-2.0 | Assinatura dos tokens | — |
| base64 | 0.22.1 | MIT OR Apache-2.0 | Codificação dos tokens | Há também 0.23.1 na árvore (transitiva) |
| getrandom | 0.3.4 | MIT OR Apache-2.0 | IDs e nonces aleatórios | Outras versões transitivas na árvore |
| rustls / tokio-rustls | 0.23.45 / 0.26.6 | Apache-2.0 OR ISC OR MIT / MIT OR Apache-2.0 | TLS do transmissor | — |
| ring | 0.17.14 | Apache-2.0 AND ISC | Criptografia do rustls | Exige manter os avisos de licença ISC |
| webpki-roots | 1.0.9 | CDLA-Permissive-2.0 | Certificados raiz para TLS | Licença de dados permissiva; exige incluir o texto da licença ao redistribuir |
| tokio-tungstenite / tungstenite | 0.29.0 / 0.29.0 | MIT / MIT OR Apache-2.0 | WebSocket do transmissor | Há também 0.30.0 na árvore: unificar versões é tarefa de limpeza |

### Rust (`crates/audio`, Etapa 4)

Licenças conferidas no crates.io em 2026-10-08. Dependências de Windows ficam em `[target.'cfg(windows)'.dependencies]`.

| Crate | Versão | Licença | Uso | Observação |
|---|---|---|---|---|
| opus | 0.4.0 | MIT OR Apache-2.0 | Binding seguro da libopus | — |
| opusic-sys (via `opus`) | 0.7.5 | BSD-3-Clause | Compila e vincula a **libopus** estaticamente (cmake) | A libopus é BSD-3-Clause (Xiph.Org e outros): manter o aviso de copyright e o texto da licença na distribuição |
| windows / windows-core | 0.62.2 | MIT OR Apache-2.0 | WASAPI, COM, Toolhelp, QPC | — |

### Ativos embutidos (Etapa 1)

| Ativo | Licença | Uso | Observação |
|---|---|---|---|
| Lucide icons 1.47.0 (9 SVGs em `apps/desktop/ui/icons/`) | ISC | Ícones da barra lateral | Texto da licença em `apps/desktop/ui/icons/LICENSE-lucide.txt` |
| Fonte | — | — | Nenhuma fonte embutida: usa a fonte do sistema (Segoe UI no Windows), custo zero no binário |

### Ferramentas de desenvolvimento (não distribuídas)

| Ferramenta | Licença | Uso |
|---|---|---|
| FFmpeg (build estático com libx264, libopus) | GPL (por causa do libx264) | Gerar mídia sintética de teste |

### Futuros (a avaliar na etapa indicada)

| Componente | Etapa | Observação |
|---|---|---|
| windows / windows-rs | 2 | MIT/Apache-2.0 (verificar) |
| Binding Opus (ex.: `opus`, `audiopus`) | 4 | Verificar licença do binding e da libopus vinculada |
| Quinn | 6 | MIT/Apache-2.0 (verificar) |
| Fonte (Inter / Noto Sans), se a fonte do sistema for insuficiente | — | OFL-1.1 — incluir texto da licença |

## 5. Obrigações a cumprir na distribuição

- Incluir `LICENSE` e, se houver, `NOTICE` no instalador e no repositório.
- Tela "Sobre" com licenças de terceiros (geradas automaticamente) e atribuição exigida pelo Slint.
- Política de CI: falhar o build se surgir dependência com licença não permitida (lista inicial permitida: MIT, Apache-2.0, BSD-2/3-Clause, ISC, Zlib, Unicode-3.0, OFL-1.1 para fontes, MPL-2.0 com análise).
- Não publicar componentes antes de concluir este inventário para eles.
