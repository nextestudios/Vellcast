# Monetização — Vellcast

> Status: **Planejamento**. Nenhuma cobrança será ativada antes da Etapa 7 e da verificação de requisitos legais e de plataforma. **Não há preços definidos.**

## 1. Modelo

Open Core + Freemium:

- **Vellcast Core** (aberto): desktop, captura, encoder, áudio, controles, protocolo, relay básico, player. Funciona com servidor próprio sem assinatura.
- **Vellcast Cloud** (comercial): servidores oficiais gerenciados, cotas maiores, recursos de comunidade, hospedagem gerenciada.

O que é cobrado é **infraestrutura e serviços gerenciados**, não o mecanismo de streaming.

## 2. Planos

### Free

- Aplicativo Windows, integração Discord.
- **Transmissão até 1920×1080 a 60 FPS** (decisão definitiva — não limitar a 720p).
- Áudio, codificação por hardware, suporte a controles.
- Salas privadas, configuração de qualidade e bitrate.
- Uso dos servidores oficiais **dentro da cota gratuita** (cota a definir após testes de capacidade e custo).
- Sem marca d'água obrigatória. Sem degradação artificial de desempenho.

### Pro

- Tudo do Free.
- 1440p60 e 4K60 quando suportados (dependem de C-001/C-002 e de hardware).
- Mais espectadores e cotas maiores no serviço oficial.
- Personalização avançada, recursos extras para transmissores, suporte prioritário.

### Community

- Tudo do Pro.
- Gerenciamento de salas e comunidades, salas permanentes, recursos administrativos.
- Capacidade ampliada de espectadores e cotas comunitárias.
- Hospedagem gerenciada.

## 3. Regras comerciais

- Não prometer tráfego ilimitado.
- Self-hosted nunca depende de assinatura para o streaming principal (W-013).
- O código aberto não bloqueia resoluções para favorecer planos pagos (W-014). Limites do serviço oficial são aplicados **no backend do Vellcast Cloud**, não no cliente.
- Lógica de planos separada de captura e encoder; o app desktop não fica mais pesado por causa do Premium.
- Benefícios pagos verificados no backend (entitlements); nunca confiar em flags enviadas pelo cliente.
- Sem processamento direto de cartão: usar provedor de pagamento adequado (ex.: Stripe, Mercado Pago, Paddle — a avaliar por elegibilidade, impostos e moedas).

## 4. Custo de banda

O relay não recodifica: cada espectador recebe a mesma mídia codificada. O custo dominante é **tráfego de saída**.

```
GB por hora por espectador = bitrate_Mbps × 3600 s ÷ 8 bits/byte ÷ 1000 MB/GB
                           = bitrate_Mbps × 0.45
GB totais = bitrate_Mbps × 0.45 × espectadores × horas   (+ entrada do transmissor: bitrate × 0.45 × horas)
```

Exemplos (vídeo + ~128 kbps de áudio arredondado no vídeo; sem overhead de protocolo/TLS, estimado em +2–5% a medir):

| Bitrate | GB/h por espectador | 5 espectadores × 2 h | 20 espectadores × 4 h |
|---|---|---|---|
| 4 Mbps | 1.8 | 18 GB | 144 GB |
| 6 Mbps | 2.7 | 27 GB | 216 GB |
| 8 Mbps (referência 1080p60) | 3.6 | 36 GB | 288 GB |
| 12 Mbps | 5.4 | 54 GB | 432 GB |

Custo por hora de sessão = GB × preço do GB de saída do provedor. Os preços de VPS/CDN variam muito (de franquia incluída a cobrança por GB); a tabela de custos real será montada na Etapa 7 com os provedores escolhidos.

Implicação: a cota gratuita deve ser expressa em **espectador-horas** ou **GB/mês**, não em "transmissões", e o bitrate máximo do Free no serviço oficial pode precisar de teto (ex.: 8 Mbps) sem limitar a resolução.

## 5. Billing e entitlements (implementado na Etapa 7, modo de teste)

Serviço: `vellcast-cloud-api` (Axum + SQLite), no repositório privado `nextestudios/vellcast-cloud` (ADR-058). Opcional: o relay self-hosted funciona sem ele (W-013).

| Peça | Como funciona |
|---|---|
| Contas | E-mail + senha (Argon2id), sessão em cookie `HttpOnly`/`SameSite=Lax` ou `Authorization: Bearer` para o app. O e-mail de `VELLCAST_CLOUD_ADMIN_EMAIL` vira administrador ao se cadastrar. |
| Planos | Free / Pro / Community com espectadores por sala, salas por conta, GB/mês e resolução/FPS máximos. **O Free nunca fica abaixo de 1080p60**, nem por edição no painel. Valores iniciais provisórios: Free 10 espectadores, 2 salas, 200 GB; Pro 50, 5, 1 TB, até 4K; Community 200, 20, 5 TB, até 4K. |
| Assinaturas | Tabela `subscriptions` (provedor + referência única). Estados: `active` (inclui `trialing`), `past_due` (mantém o plano até o fim do período pago: carência) e `canceled`. Plano efetivo = assinatura válida de maior nível; sem nenhuma, Free. |
| Entitlements | `POST /api/entitlements/token` emite um token HMAC-SHA256 de 10 min com os limites do plano (crate `vellcast-entitlement`). O relay hospedado (`VELLCAST_CLOUD_ENTITLEMENT_KEY`) exige o token no cabeçalho `X-Vellcast-Entitlement` ao criar salas e aplica espectadores por sala e salas por conta. |
| Cotas | Relays hospedados reportam tráfego por conta em `POST /api/usage` (chave de serviço). Acima da cota mensal, novos entitlements recebem `402 quota_exceeded`. |
| Pagamentos | Nunca processamos cartões. `POST /api/billing/webhooks/stripe` verifica `Stripe-Signature` (HMAC-SHA256 de `t.corpo`, tolerância de 5 min contra replay), é idempotente por ID de evento e aplica `customer.subscription.created/updated/deleted`. O checkout precisa gravar `account_id` e `plan` nos metadados da assinatura. |
| Administração | `/admin`: HTML sem JavaScript, só contas `admin`, CSRF em todos os formulários; lista contas e planos, concede plano manualmente (provedor `manual`) e ajusta limites. |

**Bloqueado para cobrança real (depende do responsável):** conta no provedor de pagamento (chaves de produção), escolha final do provedor (Stripe ou Mercado Pago para BRL/Pix), termos de uso e política de privacidade revisados juridicamente, e regras fiscais no Brasil. O serviço está pronto e testado em modo de teste.

## 6. Discord — política de monetização (pesquisa de 2026-10-08)

Fontes oficiais consultadas em 2026-10-08:
- Monetization Overview: https://docs.discord.com/developers/monetization/overview
- Enabling Monetization: https://docs.discord.com/developers/monetization/enabling-monetization
- Premium Apps & Activities: https://docs.discord.com/developers/platform/app-monetization
- Premium Apps Payouts (Help Center): https://support-dev.discord.com/hc/articles/17299902720919 (acesso automatizado bloqueado; conteúdo confirmado por buscas)
- Required Support for Monetizing Apps: https://support-dev.discord.com/hc/en-us/articles/23810643331735

| Ponto | O que a documentação diz | Impacto no Vellcast |
|---|---|---|
| Elegibilidade por região | "Premium Apps is not currently available outside of these regions": Estados Unidos, União Europeia e Reino Unido (payouts via Stripe). **O Brasil não aparece.** | A Next Estúdios (Brasil) **não é elegível hoje**. Não vender pelo checkout do Discord até isso mudar. |
| Requisitos da equipe | Dono da equipe com 18+ anos, e-mails verificados e 2FA, app verificado e pertencente à equipe, links de Termos de Serviço e Política de Privacidade, aceite dos Monetization Terms e da Developer Policy. | Mesmo com elegibilidade futura, exige verificação do app e documentos legais (Etapa 8). |
| Tipos de oferta | SKUs de assinatura (por usuário ou por servidor) e compras únicas (duráveis e consumíveis); Discord cuida de cobrança, fraude e recibos; o app concede o acesso. | Pro/Community mapeiam para assinaturas de usuário e de servidor (guild), respectivamente. |
| Activities | "App Monetization applies to Bots and Activities"; checkout nativo dentro do Discord. | Compras na Activity só via checkout do Discord, se elegível. |
| Repasse | Monetization Terms: 85% ao desenvolvedor sobre o primeiro US$ 1 milhão (15% de taxa), depois 70/30; payout após US$ 100 e revisão. | Considerar na precificação quando/se elegível. |
| Paridade e links externos | Não encontrado de forma explícita nas páginas acessadas. | **Não presumir**: antes de qualquer menção a preços ou links de compra dentro da Activity, revisar os Monetization Terms e a Developer Policy vigentes. |

Decisão até lá (ADR-043): a assinatura é vendida **fora do Discord** (site/app desktop) pela cloud-api; a Activity não exibe preços nem links de compra.

## 7. Pendências

| Item | Etapa | Status |
|---|---|---|
| Teste de capacidade do relay (espectadores por vCPU/banda) | 3 / 6 | Parcial — 20 espectadores 1080p60 por 60 min com 8% de CPU (PERFORMANCE §3.4) |
| Tabela de custos por provedor | 7 | Backlog — depende da escolha da hospedagem no Brasil |
| Definição de cotas Free | 7 | Provisória (200 GB/mês, 10 espectadores, 2 salas); revisar com custos reais |
| Elegibilidade Discord Premium Apps (Brasil) | 7 | Verificado: **não elegível** em 2026-10-08 (só EUA, UE e Reino Unido) |
| Escolha do provedor de pagamento | 7 | Bloqueado — decisão e conta do responsável; Stripe implementado em modo de teste |
| Termos de uso e política de privacidade | 8 | Etapa 8 |
