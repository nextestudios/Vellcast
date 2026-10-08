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

## 5. Billing e entitlements (arquitetura)

Preparar (Etapa 7, `services/cloud-api/`):

- Contas, planos, assinaturas, cotas, entitlements, cancelamentos, histórico comercial, gestão de comunidades.
- Webhooks do provedor de pagamento → atualização de entitlements → o relay oficial consulta limites (espectadores, bitrate, horas) via token assinado de curta duração.
- O relay self-hosted ignora entitlements: limites vêm da configuração do operador.

## 6. Discord — pontos a verificar antes de cobrar

Verificar na documentação e políticas **vigentes** do Discord (não presumir):

- **Premium Apps / monetização de Activities**: disponibilidade por país do desenvolvedor. **Não presumir que desenvolvedores no Brasil são elegíveis.** Verificar requisitos de equipe, verificação do app e localidades.
- Paridade de ofertas e de preços entre compras dentro do Discord e fora dele.
- Restrições de links/compras externas dentro de Activities.
- Requisitos de verificação do aplicativo e termos para desenvolvedores.

Se Premium Apps não for elegível, a assinatura será vendida fora do Discord (site/app desktop), respeitando as regras sobre o que pode ser mencionado dentro da Activity.

## 7. Pendências

| Item | Etapa | Status |
|---|---|---|
| Teste de capacidade do relay (espectadores por vCPU/banda) | 3 / 6 | Backlog |
| Tabela de custos por provedor | 7 | Backlog |
| Definição de cotas Free | 7 | Backlog |
| Elegibilidade Discord Premium Apps (Brasil) | 7 | Backlog |
| Escolha do provedor de pagamento | 7 | Backlog |
| Termos de uso e política de privacidade | 8 | Backlog |
