# vellcast-audio

Captura WASAPI (loopback por processo ou do sistema, excluindo o Discord) e codificação Opus em pacotes de 20 ms. **Etapa 4** — implementado, pendente de validação em Windows real.

Documentação: [`docs/architecture/AUDIO.md`](../../docs/architecture/AUDIO.md).

- Build Windows requer cmake (para a libopus).
- Verificação cruzada a partir de macOS/Linux: `cargo check --target x86_64-pc-windows-gnu -p vellcast-audio --no-default-features`.
