# vellcast-core

API do motor de transmissão usada pela interface (`apps/desktop`): `list_sources`, `list_encoders` e `StreamSession` (captura → encoder → relay, com áudio). Fora do Windows todas as operações retornam `CoreError::Unsupported`.

Implementação Windows em `src/platform/`: thread `vellcast-pipeline` (D3D11, captura, conversão, encoder), runtime Tokio de 1 worker para o transporte e áudio via `vellcast-audio`. Métricas publicadas a cada segundo em atômicos (`SessionStats`).

Teste manual no Windows, sem relay:

```text
cargo run --release -p vellcast-core --example capture_to_file -- list
cargo run --release -p vellcast-core --example capture_to_file -- 0 10 saida.h264 60 8000
ffplay saida.h264
```

Verificação cruzada no macOS (sem executar): `cargo check --target x86_64-pc-windows-gnu -p vellcast-core --no-default-features` com um toolchain mingw (ex.: llvm-mingw) em `CC_x86_64_pc_windows_gnu`, necessário para o `ring`.
