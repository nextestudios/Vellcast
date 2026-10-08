# vellcast-encoder

Codificação H.264 via **Media Foundation**. Etapa 2.

- `list_encoders()`: MFTs H.264 do sistema, hardware primeiro (NVIDIA/AMD/Intel) e depois software (Microsoft).
- `H264Encoder::new(device, config)`: tenta os candidatos até um aceitar o dispositivo D3D11 da captura. Baixa latência, CBR, GOP de 2 s, sem B-frames, perfil High (ou Main).
- `submit(textura BGRA, ...)`: converte para NV12 e escala na GPU (`ID3D11VideoProcessor`) e envia ao MFT sem bloquear. `poll()` coleta as saídas.
- `annexb`: normalização para o protocolo (SPS/PPS antes de todo IDR, sem AUD) e codec string `avc1.PPCCLL`.
- `sizing`: dimensões de saída, letterbox, GOP e bitrate.

O encoder de software só entra se não houver hardware utilizável e, se ele não aceitar Direct3D, o NV12 é copiado para memória (aviso explícito na UI).

Testes:
- `cargo test -p vellcast-encoder`: lógica pura (qualquer plataforma) e enumeração no Windows.
- `cargo test -p vellcast-encoder -- --ignored` em um Windows com GPU: codifica quadros sintéticos e verifica SPS/PPS + IDR e keyframe sob demanda.
