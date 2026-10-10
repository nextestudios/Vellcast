//! Mixagem do microfone com o áudio do aplicativo/sistema (issue #5).
//!
//! O microfone roda numa thread própria e entrega amostras já convertidas para
//! 48 kHz estéreo `f32` (conversão do Windows ou [`crate::convert::Converter`]).
//! A thread principal de áudio, a cada bloco de 20 ms, retira 960 amostras por
//! canal da [`MicFifo`] e soma ao bloco do loopback com [`mix_into`].
//!
//! A fila é um *jitter buffer* simples e limitado:
//!
//! - **Capacidade fixa**: acima dela, as amostras mais antigas são descartadas
//!   (limita a latência do microfone quando o relógio dele adianta).
//! - **Pré-carga**: depois de esvaziar (início ou falta de dados), só volta a
//!   entregar quando acumula [`MicFifo::prefill_frames`]; até lá, o bloco fica
//!   sem microfone. Isso evita picotar quando o loopback consome em rajadas.
//!
//! Lógica pura, sem alocação no caminho quente e testável em qualquer SO.

use std::collections::VecDeque;

use crate::{CHANNELS, FRAME_SAMPLES, SAMPLE_RATE};

const CH: usize = CHANNELS as usize;

/// Capacidade padrão da fila do microfone (300 ms).
pub const MIC_CAPACITY_FRAMES: usize = SAMPLE_RATE as usize * 3 / 10;
/// Pré-carga padrão antes de entregar áudio do microfone (40 ms).
pub const MIC_PREFILL_FRAMES: usize = FRAME_SAMPLES * 2;
/// Limiar a partir do qual o limitador suave comprime picos.
const KNEE: f32 = 0.8;

/// Fila limitada de amostras estéreo intercaladas do microfone.
#[derive(Debug)]
pub struct MicFifo {
    buf: VecDeque<f32>,
    capacity_frames: usize,
    prefill_frames: usize,
    primed: bool,
    dropped_frames: u64,
    underruns: u64,
}

impl MicFifo {
    /// `capacity_frames` e `prefill_frames` em amostras por canal.
    pub fn new(capacity_frames: usize, prefill_frames: usize) -> Self {
        let capacity_frames = capacity_frames.max(FRAME_SAMPLES);
        Self {
            buf: VecDeque::with_capacity(capacity_frames * CH),
            capacity_frames,
            prefill_frames: prefill_frames.min(capacity_frames),
            primed: false,
            dropped_frames: 0,
            underruns: 0,
        }
    }

    pub fn len_frames(&self) -> usize {
        self.buf.len() / CH
    }

    pub fn prefill_frames(&self) -> usize {
        self.prefill_frames
    }

    /// Amostras por canal descartadas por excesso (relógio do microfone adiantado).
    pub fn dropped_frames(&self) -> u64 {
        self.dropped_frames
    }

    /// Vezes em que o consumo esvaziou a fila (relógio do microfone atrasado).
    pub fn underruns(&self) -> u64 {
        self.underruns
    }

    /// Acrescenta amostras estéreo intercaladas, descartando as mais antigas se faltar espaço.
    pub fn push(&mut self, samples: &[f32]) {
        let samples = &samples[..samples.len() - samples.len() % CH];
        let cap = self.capacity_frames * CH;
        // Um pacote maior que a fila inteira: só o final interessa.
        let samples = if samples.len() > cap {
            self.dropped_frames += ((samples.len() - cap) / CH) as u64;
            &samples[samples.len() - cap..]
        } else {
            samples
        };
        let overflow = (self.buf.len() + samples.len()).saturating_sub(cap);
        if overflow > 0 {
            self.buf.drain(..overflow);
            self.dropped_frames += (overflow / CH) as u64;
        }
        self.buf.extend(samples.iter().copied());
    }

    /// Preenche `out` (estéreo intercalado) com o microfone. Retorna `false`, sem
    /// tocar em `out`, enquanto a fila está pré-carregando ou vazia.
    pub fn pop_into(&mut self, out: &mut [f32]) -> bool {
        let need = out.len() - out.len() % CH;
        if !self.primed {
            if self.len_frames() < self.prefill_frames.max(need / CH) {
                return false;
            }
            self.primed = true;
        }
        if self.buf.len() < need {
            // Falta de dados: descarta o resto e volta a pré-carregar.
            self.underruns += 1;
            self.buf.clear();
            self.primed = false;
            return false;
        }
        for (dst, src) in out[..need].iter_mut().zip(self.buf.drain(..need)) {
            *dst = src;
        }
        true
    }

    /// Esvazia a fila (microfone silenciado, trocado ou parado).
    pub fn clear(&mut self) {
        self.buf.clear();
        self.primed = false;
    }
}

impl Default for MicFifo {
    fn default() -> Self {
        Self::new(MIC_CAPACITY_FRAMES, MIC_PREFILL_FRAMES)
    }
}

/// Limitador suave: linear até ±0,8 e compressão contínua até ±1,0.
pub fn soft_clip(x: f32) -> f32 {
    let a = x.abs();
    if a <= KNEE || !a.is_finite() {
        return if x.is_finite() { x } else { 0.0 };
    }
    let range = 1.0 - KNEE;
    let y = KNEE + range * ((a - KNEE) / range).tanh();
    y.copysign(x)
}

/// Soma `src × gain` em `dst` (mesmo tamanho), com limitador suave.
pub fn mix_into(dst: &mut [f32], src: &[f32], gain: f32) {
    let gain = if gain.is_finite() {
        gain.clamp(0.0, 2.0)
    } else {
        1.0
    };
    for (d, s) in dst.iter_mut().zip(src) {
        *d = soft_clip(*d + s * gain);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stereo(frames: usize, v: f32) -> Vec<f32> {
        vec![v; frames * CH]
    }

    #[test]
    fn waits_for_prefill_then_delivers_blocks() {
        let mut f = MicFifo::new(MIC_CAPACITY_FRAMES, 1920);
        let mut out = stereo(FRAME_SAMPLES, 9.0);
        f.push(&stereo(960, 0.1));
        assert!(!f.pop_into(&mut out), "ainda pré-carregando");
        assert!(out.iter().all(|&s| s == 9.0), "saída intacta sem dados");
        f.push(&stereo(960, 0.2));
        assert!(f.pop_into(&mut out));
        assert!(out.iter().all(|&s| s == 0.1));
        assert!(f.pop_into(&mut out));
        assert!(out.iter().all(|&s| s == 0.2));
        assert_eq!(f.len_frames(), 0);
    }

    #[test]
    fn underrun_restarts_prefill() {
        let mut f = MicFifo::new(MIC_CAPACITY_FRAMES, 960);
        let mut out = stereo(FRAME_SAMPLES, 0.0);
        f.push(&stereo(1200, 0.3));
        assert!(f.pop_into(&mut out));
        // Sobraram 240 amostras: não dá um bloco; descarta e volta a pré-carregar.
        assert!(!f.pop_into(&mut out));
        assert_eq!(f.underruns(), 1);
        assert_eq!(f.len_frames(), 0);
        f.push(&stereo(500, 0.4));
        assert!(!f.pop_into(&mut out));
        f.push(&stereo(500, 0.4));
        assert!(f.pop_into(&mut out));
    }

    #[test]
    fn capacity_bounds_latency_by_dropping_oldest() {
        let mut f = MicFifo::new(2000, 960);
        f.push(&stereo(1500, 0.1));
        f.push(&stereo(1500, 0.2));
        assert_eq!(f.len_frames(), 2000);
        assert_eq!(f.dropped_frames(), 1000);
        let mut out = stereo(FRAME_SAMPLES, 0.0);
        assert!(f.pop_into(&mut out));
        // As 500 amostras de 0.1 mais recentes sobreviveram no início da fila.
        assert!(out[..500 * CH].iter().all(|&s| s == 0.1));
        assert!(out[500 * CH..].iter().all(|&s| s == 0.2));
    }

    #[test]
    fn huge_packet_keeps_only_the_tail() {
        let mut f = MicFifo::new(1000, 960);
        let mut data = stereo(3000, 0.1);
        data.extend(stereo(1000, 0.7));
        f.push(&data);
        assert_eq!(f.len_frames(), 1000);
        assert_eq!(f.dropped_frames(), 3000);
        let mut out = stereo(FRAME_SAMPLES, 0.0);
        assert!(f.pop_into(&mut out));
        assert!(out.iter().all(|&s| s == 0.7));
    }

    #[test]
    fn clear_discards_and_requires_prefill_again() {
        let mut f = MicFifo::new(MIC_CAPACITY_FRAMES, 960);
        f.push(&stereo(2000, 0.5));
        f.clear();
        let mut out = stereo(FRAME_SAMPLES, 0.0);
        assert!(!f.pop_into(&mut out));
    }

    #[test]
    fn ignores_trailing_odd_sample() {
        let mut f = MicFifo::default();
        let mut data = stereo(10, 0.1);
        data.push(0.9);
        f.push(&data);
        assert_eq!(f.len_frames(), 10);
    }

    #[test]
    fn soft_clip_is_transparent_below_knee_and_bounded() {
        for x in [-0.8f32, -0.3, 0.0, 0.25, 0.8] {
            assert_eq!(soft_clip(x), x);
        }
        let mut prev = soft_clip(0.8);
        let mut x = 0.81f32;
        while x < 10.0 {
            let y = soft_clip(x);
            assert!(y <= 1.0 && y >= prev, "monótono e limitado em {x}");
            assert_eq!(soft_clip(-x), -y, "simétrico em {x}");
            prev = y;
            x += 0.05;
        }
        assert_eq!(soft_clip(f32::NAN), 0.0);
        assert_eq!(soft_clip(f32::INFINITY), 0.0);
    }

    #[test]
    fn mix_sums_with_gain_and_never_exceeds_full_scale() {
        let mut dst = vec![0.2f32, -0.2, 0.9, -0.9];
        mix_into(&mut dst, &[0.1, 0.1, 0.9, -0.9], 1.0);
        assert!((dst[0] - 0.3).abs() < 1e-6);
        assert!((dst[1] + 0.1).abs() < 1e-6);
        assert!(dst[2] <= 1.0 && dst[2] > 0.9);
        assert!(dst[3] >= -1.0 && dst[3] < -0.9);

        let mut muted = vec![0.5f32; 4];
        mix_into(&mut muted, &[1.0; 4], 0.0);
        assert_eq!(muted, vec![0.5; 4]);

        let mut bad_gain = vec![0.0f32; 2];
        mix_into(&mut bad_gain, &[0.25, 0.25], f32::NAN);
        assert_eq!(bad_gain, vec![0.25, 0.25]);
    }

    #[test]
    fn mono_microphone_resampled_and_mixed_end_to_end() {
        use crate::convert::{Converter, InputFormat, SampleFormat};
        // Microfone típico: 44,1 kHz mono 16 bits, 10 ms por pacote.
        let mut conv = Converter::new(InputFormat {
            format: SampleFormat::I16,
            channels: 1,
            sample_rate: 44_100,
            channel_mask: 0,
        });
        let packet: Vec<u8> = std::iter::repeat_n(8192i16.to_le_bytes(), 441)
            .flatten()
            .collect();
        let mut fifo = MicFifo::new(MIC_CAPACITY_FRAMES, 960);
        let mut converted = Vec::new();
        for _ in 0..10 {
            converted.clear();
            conv.convert(&packet, 441, false, 1.0, &mut converted);
            fifo.push(&converted);
        }
        // 100 ms a 44,1 kHz viram ~100 ms a 48 kHz (± o atraso do reamostrador).
        assert!(
            (4700..=4800).contains(&fifo.len_frames()),
            "{}",
            fifo.len_frames()
        );
        let mut block = stereo(FRAME_SAMPLES, 0.1);
        let mut mic = stereo(FRAME_SAMPLES, 0.0);
        assert!(fifo.pop_into(&mut mic));
        mix_into(&mut block, &mic, 1.0);
        // Depois do transitório inicial do reamostrador: 0.1 + 0.25 nos dois canais.
        let tail = &block[100 * CH..];
        assert!(
            tail.iter().all(|&s| (s - 0.35).abs() < 1e-3),
            "{:?}",
            &tail[..4]
        );
    }
}
