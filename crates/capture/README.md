# vellcast-capture

Captura de janelas e monitores via **Windows Graphics Capture** (Windows 10 2004+). Etapa 2.

- `list_sources()`: janelas visíveis com título (prováveis jogos primeiro, heurística não invasiva em `heuristics.rs`) e monitores. Exclui a própria janela do Vellcast, janelas ocultas (*cloaked*) e de ferramenta.
- `D3dDevice`: dispositivo D3D11 com BGRA + vídeo e proteção multithread, compartilhado com o encoder.
- `Capturer::start(...)`: sessão WGC com pool *free-threaded*; entrega `CapturedFrame` (textura na GPU, timestamp QPC em 100 ns) em uma `LatestSlot` (capacidade 1). Recria o pool quando a fonte muda de tamanho e avisa quando ela é fechada.
- `clock`: QPC e conversão para 100 ns (mesma base do áudio).

O usuário sempre escolhe a fonte explicitamente. Sem injeção em processos, sem hooks.

Testes: `pacing`, `slot` e `heuristics` rodam em qualquer plataforma (`cargo test -p vellcast-capture`). A captura real só pode ser validada em um Windows com sessão interativa — use o exemplo `capture_to_file` (ver `crates/vellcast-core`).
