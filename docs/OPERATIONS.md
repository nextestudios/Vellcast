# Operação do serviço hospedado (runbook)

Para o servidor de testes `https://vellcast.nextestudios.com` (relay em Docker no Dokploy). Vale também para qualquer relay em Docker. Detalhes de instalação: [`DEPLOYMENT.md`](DEPLOYMENT.md).

## 1. Saúde e métricas

| Verificação | Como | Esperado |
|---|---|---|
| Processo vivo | `GET /healthz` | `200 ok` |
| Health check do container | `HEALTHCHECK` da imagem (`infrastructure/docker/relay.Dockerfile`), a cada 30 s | `healthy` no Dokploy / `docker ps` |
| Métricas | `GET /metrics` (JSON) | `rooms`, `viewers`, `bytesIn/Out`, `laggedMessages`, `staleFramesDropped`, `slowViewersDisconnected`, `authFailures`, `rateLimited` |
| Certificado | `openssl s_client -connect vellcast.nextestudios.com:443` | Let's Encrypt válido (renovação automática pelo Traefik) |

Sinais de problema:

- `laggedMessages`, `staleFramesDropped` ou `slowViewersDisconnected` crescendo → espectadores com rede lenta ou servidor sem banda.
- `authFailures` ou `rateLimited` crescendo rápido → tentativa de abuso; ver seção 4.
- Memória do container crescendo continuamente → abrir issue com `/metrics` e o gráfico do Dokploy (referência: 3–11 MB com 20 espectadores por 60 min, `PERFORMANCE.md`).

## 2. Alertas (passos no painel, feitos pelo responsável)

1. Dokploy → **Settings → Notifications** → adicionar um canal (Discord, Telegram ou e-mail).
2. Marcar **App Deploy** (sucesso e falha), **Docker Cleanup** e **Server Threshold** (CPU e memória).
3. Em **Server → Monitoring**, definir limites (sugestão: CPU 80%, memória 80%).
4. Monitor externo (opcional, recomendado): um verificador de uptime (ex.: Uptime Kuma, UptimeRobot) apontando para `https://vellcast.nextestudios.com/healthz` a cada 1 min.

## 3. Deploy e rollback

- **Deploy:** Dokploy → aplicação do relay → **Deploy** (compila a `main` pelo `infrastructure/docker/relay.Dockerfile`). O build leva ~3–5 min.
- **Verificação pós-deploy:** `/healthz`, `/watch` (player carrega) e uma transmissão curta com `node tests/benchmarks/relay-viewers.mjs https://vellcast.nextestudios.com 720p30 1 20`.
- **Rollback:** Dokploy → **Deployments** → escolher o deploy anterior → **Rollback** (ou redeploy de um commit anterior). Salas ficam em memória: **qualquer reinício encerra as salas abertas** (E3-T11); avisar os testadores.
- **Segredo do relay:** trocar `VELLCAST_RELAY_SECRET` invalida todos os tokens e convites existentes. Fazer fora de horário de teste.

## 4. Abuso e incidentes

1. Ver `rateLimited`, `authFailures` e o log do container (JSON) para identificar IPs.
2. Para fechar a criação de salas a terceiros: definir `VELLCAST_CREATE_ROOM_KEY` no ambiente e redeploy (testadores passam a informar a chave em Configurações → Servidor).
3. Bloquear IPs no firewall da VPS (`ufw deny from <ip>`) ou no Traefik.
4. Incidente de segurança: seguir [`SECURITY.md`](../SECURITY.md); registrar data, impacto e correção.

## 5. Capacidade e custos

Cada espectador consome o bitrate da transmissão em banda de **saída** do servidor (8 Mbps ≈ 3,6 GB/h por espectador). Ver [`MONETIZATION.md`](MONETIZATION.md) e a medição de região em `DEPLOYMENT.md` §6.10 (o servidor na Europa limita a vazão para o Brasil).
