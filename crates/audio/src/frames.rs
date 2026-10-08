//! Montagem de blocos de 20 ms com linha do tempo contínua.
//!
//! O WASAPI entrega pacotes de tamanho variável com o instante (QPC) da
//! primeira amostra. Aqui eles viram blocos exatos de [`FRAME_SAMPLES`]
//! amostras por canal, com PTS monotônicos:
//!
//! - lacunas (silêncio sem pacotes, descontinuidades) viram silêncio, para
//!   que o áudio continue alinhado ao vídeo;
//! - dados que chegam "antes" do esperado (sobreposição) são cortados;
//! - variações pequenas de timestamp (jitter) são ignoradas.

use crate::{CHANNELS, FRAME_SAMPLES, SAMPLE_RATE};

const CH: usize = CHANNELS as usize;
/// Diferença de timestamp tolerada antes de inserir silêncio ou cortar dados.
pub const JITTER_TOLERANCE_US: u64 = 10_000;
/// Lacunas maiores que isso reiniciam a linha do tempo em vez de gerar silêncio.
pub const MAX_GAP_FILL_US: u64 = 2_000_000;

fn us_to_samples(us: u64) -> u64 {
    (u128::from(us) * u128::from(SAMPLE_RATE) / 1_000_000) as u64
}

fn samples_to_us(samples: u64) -> u64 {
    (u128::from(samples) * 1_000_000 / u128::from(SAMPLE_RATE)) as u64
}

/// Acumula amostras estéreo intercaladas e emite blocos de 20 ms.
#[derive(Debug, Default)]
pub struct FrameAssembler {
    /// PTS (µs) da amostra 0 da linha do tempo atual.
    origin_us: Option<u64>,
    /// Índice (por canal) da primeira amostra em `buf`.
    buf_start: u64,
    /// Amostras intercaladas ainda não emitidas (sempre < 1 bloco após `push`).
    buf: Vec<f32>,
}

impl FrameAssembler {
    pub fn new() -> Self {
        Self {
            origin_us: None,
            buf_start: 0,
            buf: Vec::with_capacity(FRAME_SAMPLES * CH * 2),
        }
    }

    /// PTS em que a próxima amostra recebida é esperada, se a linha do tempo começou.
    pub fn expected_pts_us(&self) -> Option<u64> {
        self.origin_us
            .map(|o| o + samples_to_us(self.next_sample_index()))
    }

    fn next_sample_index(&self) -> u64 {
        self.buf_start + (self.buf.len() / CH) as u64
    }

    /// Adiciona amostras estéreo intercaladas cuja primeira amostra ocorre em `pts_us`.
    /// `emit` recebe cada bloco completo (`FRAME_SAMPLES * 2` valores) e seu PTS.
    pub fn push(&mut self, samples: &[f32], pts_us: u64, emit: &mut dyn FnMut(&[f32], u64)) {
        let mut samples = &samples[..samples.len() - samples.len() % CH];
        if samples.is_empty() {
            return;
        }
        let Some(origin) = self.origin_us else {
            self.origin_us = Some(pts_us);
            self.buf_start = 0;
            self.buf.clear();
            self.buf.extend_from_slice(samples);
            self.drain(emit);
            return;
        };

        let expected = origin + samples_to_us(self.next_sample_index());
        if pts_us > expected + JITTER_TOLERANCE_US {
            let gap = pts_us - expected;
            if gap > MAX_GAP_FILL_US {
                // Lacuna longa: fecha o bloco parcial com silêncio e recomeça no novo PTS.
                self.flush_with_silence(emit);
                self.origin_us = Some(pts_us);
                self.buf_start = 0;
            } else {
                self.push_silence_samples(us_to_samples(gap), emit);
            }
        } else if pts_us + JITTER_TOLERANCE_US < expected {
            // Sobreposição: descarta o trecho já coberto.
            let overlap = (us_to_samples(expected - pts_us) as usize).saturating_mul(CH);
            if overlap >= samples.len() {
                return;
            }
            samples = &samples[overlap..];
        }
        self.buf.extend_from_slice(samples);
        self.drain(emit);
    }

    /// Completa com silêncio até `pts_us` (usado quando o dispositivo não entrega
    /// pacotes, como no loopback clássico durante silêncio).
    pub fn fill_silence_until(&mut self, pts_us: u64, emit: &mut dyn FnMut(&[f32], u64)) {
        let Some(expected) = self.expected_pts_us() else {
            return;
        };
        if pts_us > expected {
            let gap = (pts_us - expected).min(MAX_GAP_FILL_US);
            self.push_silence_samples(us_to_samples(gap), emit);
        }
    }

    fn push_silence_samples(&mut self, count: u64, emit: &mut dyn FnMut(&[f32], u64)) {
        let mut remaining = count as usize * CH;
        while remaining > 0 {
            let room = FRAME_SAMPLES * CH - self.buf.len() % (FRAME_SAMPLES * CH);
            let n = remaining.min(room);
            self.buf.resize(self.buf.len() + n, 0.0);
            remaining -= n;
            self.drain(emit);
        }
    }

    fn flush_with_silence(&mut self, emit: &mut dyn FnMut(&[f32], u64)) {
        if !self.buf.is_empty() {
            self.buf.resize(FRAME_SAMPLES * CH, 0.0);
            self.drain(emit);
        }
    }

    fn drain(&mut self, emit: &mut dyn FnMut(&[f32], u64)) {
        let Some(origin) = self.origin_us else {
            return;
        };
        let block = FRAME_SAMPLES * CH;
        let mut offset = 0;
        while self.buf.len() - offset >= block {
            let pts = origin + samples_to_us(self.buf_start);
            emit(&self.buf[offset..offset + block], pts);
            offset += block;
            self.buf_start += FRAME_SAMPLES as u64;
        }
        if offset > 0 {
            self.buf.drain(..offset);
        }
    }

    /// Descarta o estado (ex.: reinício do dispositivo).
    pub fn reset(&mut self) {
        self.origin_us = None;
        self.buf_start = 0;
        self.buf.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FRAME_DURATION_US;

    fn collect(a: &mut FrameAssembler, samples: &[f32], pts: u64) -> Vec<(Vec<f32>, u64)> {
        let mut out = Vec::new();
        a.push(samples, pts, &mut |f, p| out.push((f.to_vec(), p)));
        out
    }

    fn stereo(n: usize, v: f32) -> Vec<f32> {
        vec![v; n * CH]
    }

    #[test]
    fn emits_exact_20ms_blocks_with_contiguous_pts() {
        let mut a = FrameAssembler::new();
        let mut frames = Vec::new();
        // 3 pacotes de 10 ms (480 amostras) começando em 1 s.
        for i in 0..6u64 {
            frames.extend(collect(&mut a, &stereo(480, 0.5), 1_000_000 + i * 10_000));
        }
        assert_eq!(frames.len(), 3);
        for (i, (f, p)) in frames.iter().enumerate() {
            assert_eq!(f.len(), FRAME_SAMPLES * CH);
            assert_eq!(*p, 1_000_000 + i as u64 * FRAME_DURATION_US);
        }
    }

    #[test]
    fn ignores_small_jitter() {
        let mut a = FrameAssembler::new();
        let mut frames = collect(&mut a, &stereo(960, 0.1), 0);
        // Chega 3 ms "atrasado": dentro da tolerância, continua contíguo.
        frames.extend(collect(&mut a, &stereo(960, 0.2), 23_000));
        assert_eq!(
            frames.iter().map(|f| f.1).collect::<Vec<_>>(),
            vec![0, 20_000]
        );
        assert!(frames[1].0.iter().all(|&s| s == 0.2));
    }

    #[test]
    fn fills_gaps_with_silence() {
        let mut a = FrameAssembler::new();
        let mut frames = collect(&mut a, &stereo(960, 0.3), 0);
        // Próximo pacote só em 60 ms: 40 ms de silêncio entre eles.
        frames.extend(collect(&mut a, &stereo(960, 0.4), 60_000));
        let pts: Vec<_> = frames.iter().map(|f| f.1).collect();
        assert_eq!(pts, vec![0, 20_000, 40_000, 60_000]);
        assert!(frames[1].0.iter().all(|&s| s == 0.0));
        assert!(frames[2].0.iter().all(|&s| s == 0.0));
        assert!(frames[3].0.iter().all(|&s| s == 0.4));
    }

    #[test]
    fn trims_overlapping_data() {
        let mut a = FrameAssembler::new();
        let mut frames = collect(&mut a, &stereo(960, 0.3), 0);
        // Pacote de 40 ms que começa em 0 de novo: os primeiros 20 ms já foram emitidos.
        let mut data = stereo(960, 0.9);
        data.extend(stereo(960, 0.5));
        frames.extend(collect(&mut a, &data, 0));
        assert_eq!(
            frames.iter().map(|f| f.1).collect::<Vec<_>>(),
            vec![0, 20_000]
        );
        assert!(frames[1].0.iter().all(|&s| s == 0.5));
    }

    #[test]
    fn fully_overlapping_packet_is_dropped() {
        let mut a = FrameAssembler::new();
        let _ = collect(&mut a, &stereo(1920, 0.3), 100_000);
        assert!(collect(&mut a, &stereo(480, 0.9), 100_000).is_empty());
        assert_eq!(a.expected_pts_us(), Some(140_000));
    }

    #[test]
    fn long_gap_restarts_timeline() {
        let mut a = FrameAssembler::new();
        let mut frames = collect(&mut a, &stereo(480, 0.3), 0);
        frames.extend(collect(&mut a, &stereo(960, 0.7), 10_000_000));
        // Bloco parcial fechado com silêncio em 0; nova linha do tempo em 10 s.
        assert_eq!(
            frames.iter().map(|f| f.1).collect::<Vec<_>>(),
            vec![0, 10_000_000]
        );
        assert!(frames[0].0[..480 * CH].iter().all(|&s| s == 0.3));
        assert!(frames[0].0[480 * CH..].iter().all(|&s| s == 0.0));
    }

    #[test]
    fn fill_silence_until_keeps_stream_flowing() {
        let mut a = FrameAssembler::new();
        let mut out = Vec::new();
        a.push(&stereo(960, 0.2), 0, &mut |_, p| out.push(p));
        a.fill_silence_until(100_000, &mut |_, p| out.push(p));
        assert_eq!(out, vec![0, 20_000, 40_000, 60_000, 80_000]);
        // Antes de qualquer dado, não há linha do tempo para preencher.
        let mut b = FrameAssembler::new();
        b.fill_silence_until(100_000, &mut |_, p| out.push(p));
        assert_eq!(out.len(), 5);
    }

    #[test]
    fn pts_are_monotonic_under_random_input() {
        let mut a = FrameAssembler::new();
        let mut last = None;
        let mut pts = 500_000u64;
        // Sequência determinística com jitter, lacunas e sobreposições.
        for i in 0..2000u64 {
            let n = 100 + (i * 37 % 900) as usize;
            let jitter = (i * 7919 % 30_000) as i64 - 15_000;
            let p = (pts as i64 + jitter).max(0) as u64;
            a.push(&stereo(n, 0.1), p, &mut |f, t| {
                assert_eq!(f.len(), FRAME_SAMPLES * CH);
                if let Some(l) = last {
                    assert!(t > l, "{t} <= {l}");
                }
                last = Some(t);
            });
            pts += samples_to_us(n as u64);
        }
        assert!(last.is_some());
    }

    #[test]
    fn ignores_trailing_odd_sample() {
        let mut a = FrameAssembler::new();
        let mut data = stereo(960, 0.1);
        data.push(0.9);
        assert_eq!(collect(&mut a, &data, 0).len(), 1);
    }
}
