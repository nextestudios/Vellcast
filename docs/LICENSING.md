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

### Ferramentas de desenvolvimento (não distribuídas)

| Ferramenta | Licença | Uso |
|---|---|---|
| FFmpeg (build estático com libx264, libopus) | GPL (por causa do libx264) | Gerar mídia sintética de teste |

### Futuros (a avaliar na etapa indicada)

| Componente | Etapa | Observação |
|---|---|---|
| Slint | 1 | Ver §2 |
| GilRs | 1 | MIT/Apache-2.0 (verificar) |
| windows / windows-rs | 2 | MIT/Apache-2.0 (verificar) |
| Binding Opus (ex.: `opus`, `audiopus`) | 4 | Verificar licença do binding e da libopus vinculada |
| Quinn | 6 | MIT/Apache-2.0 (verificar) |
| Fonte (Inter / Noto Sans) | 1 | OFL-1.1 — incluir texto da licença |
| Ícones (ex.: Lucide) | 1 | ISC (verificar) |

## 5. Obrigações a cumprir na distribuição

- Incluir `LICENSE` e, se houver, `NOTICE` no instalador e no repositório.
- Tela "Sobre" com licenças de terceiros (geradas automaticamente) e atribuição exigida pelo Slint.
- Política de CI: falhar o build se surgir dependência com licença não permitida (lista inicial permitida: MIT, Apache-2.0, BSD-2/3-Clause, ISC, Zlib, Unicode-3.0, OFL-1.1 para fontes, MPL-2.0 com análise).
- Não publicar componentes antes de concluir este inventário para eles.
