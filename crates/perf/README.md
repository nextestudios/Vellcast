# vellcast-perf

Ferramentas de medição reproduzíveis (não distribuídas). Sempre em `--release`.

| Binário | O que mede |
|---|---|
| `fanout_bench` | Relay em processo: custo da distribuição para N espectadores. Alocações e bytes alocados **apenas nas threads do relay** (alocador contador), CPU do processo, FPS e latência vistos pelos espectadores. Compara escrita em lote (`batch=on`) e uma escrita por mensagem (`batch=off`). |
| `quic_eval` | QUIC (Quinn, um stream por quadro) × WebSocket/TCP com perda, atraso, jitter e apagão simulados por um proxy no mesmo processo. Latência por quadro, quadros entregues, CPU, RSS e recuperação após apagão. |

```bash
cargo run --release -p vellcast-perf --bin fanout_bench -- 20 20 60 8000 on
cargo run --release -p vellcast-perf --bin fanout_bench -- 20 20 60 8000 off
cargo run --release -p vellcast-perf --bin quic_eval -- 10
```

Benchmarks de ponta a ponta com processos separados ficam em `tests/benchmarks/` (`relay-viewers.mjs`, `stage0-viewers.mjs`) e o E2E Windows em `tests/e2e/`. Metodologia e resultados: `docs/PERFORMANCE.md`.

Limitações: tudo roda na mesma máquina (loopback). A CPU é do processo inteiro (relay e clientes). No `quic_eval`, a perda no TCP é modelada como atraso de retransmissão, porque segmentos TCP não podem ser descartados no espaço do usuário.
