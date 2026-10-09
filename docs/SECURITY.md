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

## 3. Autenticação e autorização
- **Activity:** `discordSdk.authorize()` → `code` → o backend troca pelo token → o backend obtém a identidade e emite um **token de sessão Vellcast** curto.
- **Desktop:** login no serviço (oficial ou self-hosted) e pareamento com a Activity por **código temporário** (TTL de minutos, uso único, tentativas limitadas).
- **Relay:** cada conexão WS apresenta um token. O relay valida assinatura, expiração, sala e papel (publisher/viewer) antes de aceitar qualquer mídia.
- **Replay:** tokens de pareamento são de uso único e tokens de sessão expiram. O nonce/sequence detecta duplicação quando aplicável.

### 3.1 Relay — implementado na ETAPA 3

| Ameaça | Mitigação implementada | Onde |
|---|---|---|
| Espectador sem convite assiste | Salas privadas por padrão; acesso só com token de espectador obtido por convite | `api.rs` (`join_room`), `ws.rs` (`authenticate`) |
| Token forjado ou adulterado | HMAC-SHA256 com segredo ≥ 32 bytes; assinatura verificada em tempo constante antes de interpretar o payload | `token.rs` |
| Token em logs ou histórico | Token só em `Authorization: Bearer` ou na mensagem `Auth`; nunca na URL; nunca registrado em log; publicador de exemplo lê o token de variável de ambiente | `api.rs`, `ws.rs`, `examples/publish_fixture.rs` |
| Token de uma sala usado em outra / papel errado | Sala e papel conferidos em toda chamada da API e no `Auth` | `api.rs` (`authorize`), `ws.rs` |
| Convite vazado ou reutilizado | Código aleatório de 128 bits, guardado só como SHA-256, com limite de usos e expiração consumidos de forma atômica; até 100 convites ativos por sala | `store.rs` |
| Segundo transmissor sequestra a sala | Um transmissor por sala; só o **mesmo** token (mesmo `nonce`) assume a vaga (reconexão); outro token recebe `publisher_exists` | `room.rs` (`attach_publisher`) |
| Criação de salas em massa | `VELLCAST_CREATE_ROOM_KEY` opcional (comparação em tempo constante), limite de salas, rate limit por IP | `api.rs`, `config.rs` |
| Força bruta / flood | Token bucket por IP na API e nas conexões WebSocket, em memória e com número de IPs rastreados limitado | `ratelimit.rs` |
| Conexão que nunca autentica | `Auth` obrigatório como primeira mensagem em 5 s, máximo de 2 KiB | `ws.rs` |
| Exaustão de memória | Filas limitadas por espectador, cache de GOP limitado em bytes, mensagens limitadas (4 MiB transmissor, 1 KiB espectador), espectador travado desconectado após 10 s | `room.rs`, `ws.rs`, `config.rs` |
| Transmissor abusando de banda | Token bucket de bytes de entrada (40 Mbps padrão) → `bitrate_exceeded` | `ws.rs` |
| Payload malicioso para espectadores | `VideoInit`/`AudioInit` validados como JSON com limites antes de repassar; tipos desconhecidos rejeitados | `ws.rs` |
| Retenção de dados | Mídia nunca é gravada. Por padrão nada vai para disco; com `VELLCAST_DB_PATH`, só metadados (salas, convites como hash SHA-256, nonces revogados) em SQLite, removidos por TTL | `store.rs`, `store_sqlite.rs` |
| Token vazado | Revogação individual por `nonce` e revogação em massa de espectadores pelo dono (`POST /api/rooms/{id}/revoke`); conexões ativas são encerradas na hora; verificada na API e no `Auth` | `api.rs` (`revoke`), `ws.rs`, `store.rs` |
| Rotação do segredo | `VELLCAST_RELAY_SECRET_PREVIOUS` aceito só na verificação; tokens novos sempre com o segredo atual | `token.rs`, `config.rs` |
| Relay sem segredo configurado | Recusa iniciar sem `VELLCAST_RELAY_SECRET`, exceto com flag explícita de desenvolvimento | `config.rs` |

**Replay:** tokens são credenciais *bearer* com expiração. Reutilizar o mesmo token de espectador ou transmissor é intencional (é o que permite a reconexão). Os itens de uso único (convites) são consumidos no servidor. **Limitações conhecidas:** a revogação em massa cobre só espectadores (tokens de dono e de transmissor são revogados individualmente; o próprio token de dono não pode ser revogado por ele mesmo — encerre a sala); no modo em memória as revogações somem ao reiniciar o relay; não há contas de usuário no relay (o "dono" é quem tem o token de dono). Contas, identidade Discord e pareamento entram na ETAPA 5.

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
