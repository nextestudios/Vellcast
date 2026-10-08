//! Codificação Opus de blocos de 20 ms (48 kHz, estéreo).

use crate::{AudioError, CHANNELS, FRAME_SAMPLES};

/// Maior pacote Opus possível (RFC 6716 §3.4: 1275 bytes por quadro, até 3 quadros + cabeçalho).
#[cfg_attr(not(feature = "opus"), allow(dead_code))]
const MAX_PACKET: usize = 4000;

pub struct OpusEncoder {
    #[cfg(feature = "opus")]
    inner: opus::Encoder,
    out: Vec<u8>,
}

impl OpusEncoder {
    #[cfg(feature = "opus")]
    pub fn new() -> Result<Self, AudioError> {
        let err = |e: opus::Error| AudioError::Encoder(e.to_string());
        // `Audio` prioriza fidelidade (música e efeitos de jogo); o atraso
        // algorítmico (~6,5 ms) é desprezível perto do jitter buffer do player.
        let mut inner = opus::Encoder::new(
            crate::SAMPLE_RATE,
            opus::Channels::Stereo,
            opus::Application::Audio,
        )
        .map_err(err)?;
        inner
            .set_bitrate(opus::Bitrate::Bits(crate::OPUS_BITRATE_BPS))
            .map_err(err)?;
        Ok(Self {
            inner,
            out: vec![0; MAX_PACKET],
        })
    }

    #[cfg(not(feature = "opus"))]
    pub fn new() -> Result<Self, AudioError> {
        Err(AudioError::Encoder(
            "compilado sem a feature `opus`; a captura de áudio não pode codificar".into(),
        ))
    }

    /// Codifica um bloco de `FRAME_SAMPLES` amostras estéreo intercaladas.
    pub fn encode(&mut self, frame: &[f32]) -> Result<Vec<u8>, AudioError> {
        if frame.len() != FRAME_SAMPLES * usize::from(CHANNELS) {
            return Err(AudioError::Encoder(format!(
                "bloco com {} amostras; esperado {}",
                frame.len(),
                FRAME_SAMPLES * usize::from(CHANNELS)
            )));
        }
        #[cfg(feature = "opus")]
        {
            let n = self
                .inner
                .encode_float(frame, &mut self.out)
                .map_err(|e| AudioError::Encoder(e.to_string()))?;
            Ok(self.out[..n].to_vec())
        }
        #[cfg(not(feature = "opus"))]
        {
            let _ = &self.out;
            Err(AudioError::Encoder("compilado sem a feature `opus`".into()))
        }
    }
}
