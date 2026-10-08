# Modelo de segurança e privacidade

## 1. Princípios
1. **Consentimento explícito.** A captura só começa por ação do usuário e com uma fonte escolhida por ele. Não existe captura escondida (W-007).
2. **Nada é gravado por padrão.** O relay não persiste mídia (W-008). Não há análise de conteúdo transmitido.
3. **Sem intrusão em jogos.** Nada de injeção, hooks em processos ou interferência com anti-cheat (W-011, W-012). Só APIs oficiais do Windows (WGC, WASAPI).
4. **O servidor é a fonte de verdade.** Identidade, permissões e planos são validados no backend. Flags vindas do cliente (incluindo "Premium") nunca são confiáveis.
5. **Retenção mínima.** Logs não contêm tokens, payloads de mídia nem conteúdo confidencial.

## 2. Ativos e ameaças

| Ativo | Ameaça | Mitigação |
|---|---|---|
| Conteúdo da transmissão | Espectador não autorizado assiste | Salas privadas por padrão, token de acesso por sala, validação no relay a cada conexão |
| Tokens de sessão/pareamento | Vazamento ou replay | Tokens temporários de uso limitado, TTL curto, vinculados à sala/usuário, nunca em logs nem na URL além do necessário |
| Credenciais Discord | Roubo | O usuário nunca compartilha token do Discord; a troca OAuth `code → token` acontece no backend, com o client secret em variável de ambiente |
| Relay | DoS/abuso | Rate limiting, limite de tamanho de mensagem (4 MiB), limites de salas/espectadores/banda, timeouts de heartbeat |
| Relay | Payload malformado | Parser de cabeçalho com validação de versão, tipo, tamanho e flags; mensagens inválidas encerram a conexão |
| Memória do relay/desktop | Exaustão | Filas sempre limitadas, descarte de frames antigos, sem buffers que crescem sem limite |
| Máquina do usuário | Captura indevida | Indicador visível de transmissão ativa, parar com um clique ou botão, fonte exibida na UI |
| Áudio | Captura da voz do Discord por engano | Loopback por processo como padrão; loopback do sistema só com aviso claro |

## 3. Autenticação e autorização (alvo: ETAPA 5)
- **Activity:** `discordSdk.authorize()` → `code` → o backend troca pelo token → o backend obtém a identidade e emite um **token de sessão Vellcast** curto.
- **Desktop:** login no serviço (oficial ou self-hosted) e pareamento com a Activity por **código temporário** (TTL de minutos, uso único, tentativas limitadas).
- **Relay:** cada conexão WS apresenta um token. O relay valida assinatura, expiração, sala e papel (publisher/viewer) antes de aceitar qualquer mídia.
- **Replay:** tokens de pareamento são de uso único e tokens de sessão expiram. O nonce/sequence detecta duplicação quando aplicável.

> **ETAPA 0:** o servidor de validação (`stage0-server`) **não tem autenticação**. Ele serve apenas mídia sintética e não deve ficar exposto além do tempo de teste. Isso é uma limitação aceita e documentada.

## 4. Transporte
- TLS obrigatório fora do ambiente local (o proxy reverso termina o TLS; o Discord exige HTTPS).
- Mensagens binárias com tamanho máximo verificado **antes** de alocar.
- Heartbeat: uma conexão sem tráfego por 15 s é encerrada.
- QUIC (se adotado): TLS 1.3 nativo, mesma camada de autorização.

## 5. Segredos
- Somente em variáveis de ambiente ou no gerenciador de segredos do Dokploy. Nunca no repositório.
- `.env` fica no `.gitignore`. É fornecido apenas um `.env.example` sem valores reais.
- Nenhuma credencial fictícia é apresentada como real.

## 6. Pagamentos (futuro)
- Sem processamento direto de cartão. O processamento é feito por um provedor certificado.
- Webhooks validados por assinatura, idempotentes, com o resultado aplicado só no backend.
- Os entitlements são resolvidos no servidor. O cliente apenas exibe.

## 7. Cadeia de suprimentos
- Versões fixadas (`Cargo.lock` e lockfile npm), de preferência publicadas há pelo menos duas semanas.
- `cargo audit` e `npm audit` no CI (a adicionar).
- Inventário de licenças de terceiros em [LICENSING.md](LICENSING.md).
- O instalador e o binário Windows serão assinados quando houver certificado (ETAPA 8). Atualizações serão verificadas por assinatura.

## 8. Reporte de vulnerabilidades
Até existir um canal dedicado, envie o reporte de forma privada para a Next Estúdios (https://nextestudios.com). Não abra issue pública para vulnerabilidades.
