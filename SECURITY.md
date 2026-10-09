# Política de segurança

## Como reportar uma vulnerabilidade

**Não abra uma issue pública.** Use o reporte privado do GitHub:

1. Acesse a aba **Security** do repositório.
2. Clique em **Report a vulnerability** (GitHub Private Vulnerability Reporting).
3. Descreva o problema, a versão ou commit afetado e, se possível, como reproduzir.

## O que esperar

| Etapa | Prazo-alvo |
|---|---|
| Confirmação do recebimento | até 3 dias úteis |
| Avaliação inicial e severidade | até 10 dias úteis |
| Correção de falhas críticas ou altas | o mais rápido possível, com aviso de segurança publicado após a correção |

Pedimos que a vulnerabilidade não seja divulgada antes da correção. Damos crédito a quem reportar, se desejar.

## Escopo

- Aplicativo desktop (`apps/desktop`), servidor de mídia (`services/relay`), player web e Discord Activity (`apps/discord-activity`), protocolo e bibliotecas em `crates/`.
- Infraestrutura hospedada pela Next Estúdios (ex.: servidor de testes).

Fora do escopo: ataques que exigem acesso físico ao computador da vítima ou privilégios de administrador, engenharia social e negação de serviço por volume.

## Versões suportadas

Enquanto o Vellcast estiver em desenvolvimento (antes da versão 1.0), apenas a versão mais recente de `main` recebe correções.

Modelo de ameaças: [`docs/SECURITY.md`](docs/SECURITY.md). Última revisão: [`docs/security/REVIEW-2026-10.md`](docs/security/REVIEW-2026-10.md).
