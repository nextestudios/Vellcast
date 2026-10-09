//! Conversão de formato para 48 kHz estéreo `f32`.
//!
//! Usada só quando o WASAPI não aceita converter sozinho
//! (`AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM`) e o dispositivo entrega o
//! formato de mixagem nativo (ex.: 44,1 kHz, 16 bits, 5.1).

use crate::SAMPLE_RATE;

/// Formato das amostras de entrada.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SampleFormat {
    F32,
    I16,
    /// 24 bits em 3 bytes.
    I24,
    /// 24 bits válidos em contêiner de 32 bits, ou 32 bits inteiros.
    I32,
}

impl SampleFormat {
    pub fn bytes(self) -> usize {
        match self {
            Self::I16 => 2,
            Self::I24 => 3,
            Self::F32 | Self::I32 => 4,
        }
    }
}

/// Descrição do formato de entrada.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputFormat {
    pub format: SampleFormat,
    pub channels: u16,
    pub sample_rate: u32,
    /// Máscara de canais (`dwChannelMask`); 0 = ordem padrão.
    pub channel_mask: u32,
}

impl InputFormat {
    pub fn is_native(&self) -> bool {
        self.format == SampleFormat::F32 && self.channels == 2 && self.sample_rate == SAMPLE_RATE
    }

    pub fn frame_bytes(&self) -> usize {
        self.format.bytes() * usize::from(self.channels)
    }
}

// Bits de posição de alto-falante (ksmedia.h).
const SPEAKER_FRONT_LEFT: u32 = 0x1;
const SPEAKER_FRONT_RIGHT: u32 = 0x2;
const SPEAKER_FRONT_CENTER: u32 = 0x4;
const SPEAKER_LOW_FREQUENCY: u32 = 0x8;
const SPEAKER_BACK_LEFT: u32 = 0x10;
const SPEAKER_BACK_RIGHT: u32 = 0x20;
const SPEAKER_FRONT_LEFT_OF_CENTER: u32 = 0x40;
const SPEAKER_FRONT_RIGHT_OF_CENTER: u32 = 0x80;
const SPEAKER_BACK_CENTER: u32 = 0x100;
const SPEAKER_SIDE_LEFT: u32 = 0x200;
const SPEAKER_SIDE_RIGHT: u32 = 0x400;

const MINUS_3DB: f32 = std::f32::consts::FRAC_1_SQRT_2;

/// Máscara padrão para quando o formato não informa uma.
fn default_mask(channels: u16) -> u32 {
    match channels {
        1 => SPEAKER_FRONT_CENTER,
        2 => SPEAKER_FRONT_LEFT | SPEAKER_FRONT_RIGHT,
        4 => SPEAKER_FRONT_LEFT | SPEAKER_FRONT_RIGHT | SPEAKER_BACK_LEFT | SPEAKER_BACK_RIGHT,
        6 => 0x3F,
        8 => 0x63F,
        _ => 0,
    }
}

/// Peso (esquerda, direita) de cada posição de alto-falante no downmix estéreo.
fn speaker_weights(bit: u32) -> (f32, f32) {
    match bit {
        SPEAKER_FRONT_LEFT => (1.0, 0.0),
        SPEAKER_FRONT_RIGHT => (0.0, 1.0),
        SPEAKER_FRONT_CENTER | SPEAKER_BACK_CENTER => (MINUS_3DB, MINUS_3DB),
        SPEAKER_LOW_FREQUENCY => (0.0, 0.0),
        SPEAKER_BACK_LEFT | SPEAKER_SIDE_LEFT | SPEAKER_FRONT_LEFT_OF_CENTER => (MINUS_3DB, 0.0),
        SPEAKER_BACK_RIGHT | SPEAKER_SIDE_RIGHT | SPEAKER_FRONT_RIGHT_OF_CENTER => (0.0, MINUS_3DB),
        _ => (0.5, 0.5),
    }
}

/// Matriz de downmix: um par (L, R) por canal de entrada, normalizada para não saturar.
pub fn downmix_matrix(channels: u16, channel_mask: u32) -> Vec<(f32, f32)> {
    if channels == 1 {
        return vec![(1.0, 1.0)];
    }
    let mask = if channel_mask != 0 {
        channel_mask
    } else {
        default_mask(channels)
    };
    let mut bits = (0..32).map(|i| 1u32 << i).filter(|b| mask & b != 0);
    let mut m: Vec<(f32, f32)> = (0..channels)
        .map(|_| bits.next().map_or((0.5, 0.5), speaker_weights))
        .collect();
    let (sl, sr) = m.iter().fold((0.0, 0.0), |(a, b), (l, r)| (a + l, b + r));
    let norm = sl.max(sr).max(1.0);
    for w in &mut m {
        w.0 /= norm;
        w.1 /= norm;
    }
    m
}

fn read_sample(bytes: &[u8], format: SampleFormat) -> f32 {
    match format {
        SampleFormat::F32 => f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
        SampleFormat::I16 => f32::from(i16::from_le_bytes([bytes[0], bytes[1]])) / 32_768.0,
        SampleFormat::I24 => {
            let v = i32::from_le_bytes([0, bytes[0], bytes[1], bytes[2]]) >> 8;
            v as f32 / 8_388_608.0
        }
        SampleFormat::I32 => {
            i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as f32 / 2_147_483_648.0
        }
    }
}

/// Converte quadros intercalados de qualquer formato para estéreo `f32` (mesma taxa).
pub struct Converter {
    input: InputFormat,
    matrix: Vec<(f32, f32)>,
    resampler: Resampler,
    scratch: Vec<f32>,
}

impl Converter {
    pub fn new(input: InputFormat) -> Self {
        Self {
            input,
            matrix: downmix_matrix(input.channels, input.channel_mask),
            resampler: Resampler::new(input.sample_rate, SAMPLE_RATE),
            scratch: Vec::new(),
        }
    }

    pub fn input(&self) -> InputFormat {
        self.input
    }

    /// Converte `data` (bytes intercalados) e acrescenta o resultado em `out`.
    /// `silent` substitui o conteúdo por silêncio (flag `AUDCLNT_BUFFERFLAGS_SILENT`).
    pub fn convert(
        &mut self,
        data: &[u8],
        frames: usize,
        silent: bool,
        gain: f32,
        out: &mut Vec<f32>,
    ) {
        self.scratch.clear();
        self.scratch.reserve(frames * 2);
        let fb = self.input.frame_bytes();
        let sb = self.input.format.bytes();
        let usable = frames.min(data.len() / fb.max(1));
        if silent {
            self.scratch.resize(frames * 2, 0.0);
        } else {
            for frame in data[..usable * fb].chunks_exact(fb) {
                let (mut l, mut r) = (0.0f32, 0.0f32);
                for (ch, (wl, wr)) in self.matrix.iter().enumerate() {
                    let s = read_sample(&frame[ch * sb..], self.input.format);
                    l += s * wl;
                    r += s * wr;
                }
                self.scratch.push((l * gain).clamp(-1.0, 1.0));
                self.scratch.push((r * gain).clamp(-1.0, 1.0));
            }
        }
        self.resampler.process(&self.scratch, out);
    }

    pub fn reset(&mut self) {
        self.resampler.reset();
    }
}

/// Reamostragem linear estéreo, contínua entre chamadas.
///
/// Suficiente para voz e jogos em transmissão (o caminho preferido é a
/// conversão do próprio Windows). Não é um conversor de alta fidelidade.
pub struct Resampler {
    in_rate: u32,
    out_rate: u32,
    /// Posição fracionária da próxima amostra de saída, em amostras de entrada,
    /// relativa ao início do bloco atual (pode ser negativa: entre `last` e o bloco).
    pos: f64,
    last: Option<(f32, f32)>,
}

impl Resampler {
    pub fn new(in_rate: u32, out_rate: u32) -> Self {
        Self {
            in_rate,
            out_rate,
            pos: 0.0,
            last: None,
        }
    }

    pub fn reset(&mut self) {
        self.pos = 0.0;
        self.last = None;
    }

    /// `input`: estéreo intercalado na taxa de entrada. Acrescenta a saída em `out`.
    pub fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        if self.in_rate == self.out_rate {
            out.extend_from_slice(input);
            return;
        }
        let n = input.len() / 2;
        if n == 0 {
            return;
        }
        let step = f64::from(self.in_rate) / f64::from(self.out_rate);
        let frame = |i: isize, last: Option<(f32, f32)>| -> (f32, f32) {
            if i < 0 {
                last.unwrap_or((input[0], input[1]))
            } else {
                let i = i as usize;
                (input[2 * i], input[2 * i + 1])
            }
        };
        // `pos` é relativo à amostra 0 deste bloco; -1 representa `last`.
        // Posições em [n-1, n) dependem da próxima amostra e ficam para o próximo bloco.
        while self.pos < (n - 1) as f64 {
            let base = self.pos.floor();
            let frac = (self.pos - base) as f32;
            let i0 = base as isize;
            let a = frame(i0, self.last);
            let b = frame(i0 + 1, self.last);
            out.push(a.0 + (b.0 - a.0) * frac);
            out.push(a.1 + (b.1 - a.1) * frac);
            self.pos += step;
        }
        self.pos -= n as f64;
        self.last = Some((input[2 * (n - 1)], input[2 * (n - 1) + 1]));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes_f32(v: &[f32]) -> Vec<u8> {
        v.iter().flat_map(|s| s.to_le_bytes()).collect()
    }

    #[test]
    fn native_format_passes_through() {
        let fmt = InputFormat {
            format: SampleFormat::F32,
            channels: 2,
            sample_rate: 48_000,
            channel_mask: 0,
        };
        assert!(fmt.is_native());
        let mut c = Converter::new(fmt);
        let mut out = Vec::new();
        c.convert(&bytes_f32(&[0.1, -0.2, 0.3, -0.4]), 2, false, 1.0, &mut out);
        assert_eq!(out, vec![0.1, -0.2, 0.3, -0.4]);
    }

    #[test]
    fn decodes_integer_formats() {
        let i16s: Vec<u8> = [i16::MAX, i16::MIN]
            .iter()
            .flat_map(|s| s.to_le_bytes())
            .collect();
        let mut c = Converter::new(InputFormat {
            format: SampleFormat::I16,
            channels: 2,
            sample_rate: 48_000,
            channel_mask: 0,
        });
        let mut out = Vec::new();
        c.convert(&i16s, 1, false, 1.0, &mut out);
        assert!((out[0] - 1.0).abs() < 1e-4 && (out[1] + 1.0).abs() < 1e-6);

        // 24 bits: 0x400000 = 0.5.
        assert!((read_sample(&[0x00, 0x00, 0x40], SampleFormat::I24) - 0.5).abs() < 1e-6);
        assert!((read_sample(&[0x00, 0x00, 0xC0], SampleFormat::I24) + 0.5).abs() < 1e-6);
        assert!((read_sample(&0x4000_0000i32.to_le_bytes(), SampleFormat::I32) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn mono_is_duplicated() {
        let mut c = Converter::new(InputFormat {
            format: SampleFormat::F32,
            channels: 1,
            sample_rate: 48_000,
            channel_mask: 0,
        });
        let mut out = Vec::new();
        c.convert(&bytes_f32(&[0.25]), 1, false, 1.0, &mut out);
        assert_eq!(out, vec![0.25, 0.25]);
    }

    #[test]
    fn surround_downmix_keeps_sides_and_drops_lfe() {
        // 5.1: FL FR FC LFE BL BR
        let m = downmix_matrix(6, 0);
        assert_eq!(m.len(), 6);
        assert_eq!(m[3], (0.0, 0.0), "LFE");
        assert!(m[0].0 > 0.0 && m[0].1 == 0.0, "FL só na esquerda");
        assert!(m[5].1 > 0.0 && m[5].0 == 0.0, "BR só na direita");
        // Soma de pesos por lado ≤ 1: não satura com todos os canais em 1.0.
        let (l, r) = m.iter().fold((0.0, 0.0), |(a, b), w| (a + w.0, b + w.1));
        assert!(l <= 1.0 + 1e-6 && r <= 1.0 + 1e-6);
    }

    #[test]
    fn silent_flag_and_gain() {
        let fmt = InputFormat {
            format: SampleFormat::F32,
            channels: 2,
            sample_rate: 48_000,
            channel_mask: 0,
        };
        let mut c = Converter::new(fmt);
        let mut out = Vec::new();
        c.convert(&bytes_f32(&[0.5, 0.5]), 1, true, 1.0, &mut out);
        assert_eq!(out, vec![0.0, 0.0]);
        out.clear();
        c.convert(&bytes_f32(&[0.4, 0.8]), 1, false, 2.0, &mut out);
        assert_eq!(out, vec![0.8, 1.0], "ganho com limite em 1.0");
    }

    #[test]
    fn resampler_ratio_and_continuity() {
        let mut r = Resampler::new(44_100, 48_000);
        let mut out = Vec::new();
        // 1 s de rampa em blocos de tamanhos variados.
        let total = 44_100usize;
        let ramp: Vec<f32> = (0..total)
            .flat_map(|i| {
                let v = i as f32 / total as f32;
                [v, -v]
            })
            .collect();
        let mut i = 0;
        let mut size = 37;
        while i < total {
            let end = (i + size).min(total);
            r.process(&ramp[2 * i..2 * end], &mut out);
            i = end;
            size = size * 7 % 1000 + 1;
        }
        let frames = out.len() / 2;
        assert!((47_990..=48_001).contains(&frames), "{frames}");
        // Rampa continua monotônica nas fronteiras dos blocos.
        for w in out.as_chunks::<2>().0.iter().collect::<Vec<_>>().windows(2) {
            assert!(w[1][0] >= w[0][0] - 1e-6);
        }
    }

    #[test]
    fn resampler_same_rate_is_identity() {
        let mut r = Resampler::new(48_000, 48_000);
        let mut out = Vec::new();
        r.process(&[0.1, 0.2, 0.3, 0.4], &mut out);
        assert_eq!(out, vec![0.1, 0.2, 0.3, 0.4]);
    }

    #[test]
    fn truncated_buffer_does_not_panic() {
        let mut c = Converter::new(InputFormat {
            format: SampleFormat::I16,
            channels: 2,
            sample_rate: 48_000,
            channel_mask: 0,
        });
        let mut out = Vec::new();
        c.convert(&[1, 2, 3], 10, false, 1.0, &mut out);
        assert!(out.is_empty());
    }
}
