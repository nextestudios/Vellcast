//! Parsers de H.264 Annex B e Ogg Opus.

use anyhow::{Context, Result, bail};
use bytes::Bytes;

const START_CODE: [u8; 4] = [0, 0, 0, 1];

const NAL_SLICE: u8 = 1;
const NAL_IDR: u8 = 5;
const NAL_SPS: u8 = 7;
const NAL_PPS: u8 = 8;
const NAL_AUD: u8 = 9;

/// Uma unidade de acesso H.264 (um quadro) em Annex B.
#[derive(Debug, Clone)]
pub struct AccessUnit {
    pub data: Bytes,
    pub keyframe: bool,
}

#[derive(Debug)]
pub struct H264Stream {
    pub access_units: Vec<AccessUnit>,
    /// Codec string WebCodecs, ex.: `avc1.64002A`.
    pub codec: String,
}

/// Divide um fluxo Annex B em NAL units (sem start codes).
pub fn split_nal_units(data: &[u8]) -> Vec<&[u8]> {
    let mut starts = Vec::new();
    let mut i = 0;
    while i + 3 <= data.len() {
        if data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1 {
            starts.push(i + 3);
            i += 3;
        } else {
            i += 1;
        }
    }
    let mut nals = Vec::with_capacity(starts.len());
    for (n, &start) in starts.iter().enumerate() {
        let mut end = starts.get(n + 1).map_or(data.len(), |&next| next - 3);
        // Remove o zero extra de start codes de 4 bytes e zeros finais.
        while end > start && data[end - 1] == 0 {
            end -= 1;
        }
        if end > start {
            nals.push(&data[start..end]);
        }
    }
    nals
}

fn nal_type(nal: &[u8]) -> u8 {
    nal[0] & 0x1F
}

fn is_vcl(t: u8) -> bool {
    (NAL_SLICE..=NAL_IDR).contains(&t)
}

/// Agrupa NAL units em unidades de acesso.
///
/// Usa AUD (tipo 9) como delimitador quando presente; caso contrário,
/// inicia um novo quadro em SPS ou quando um VCL aparece após outro VCL
/// (válido para fluxos com uma slice por quadro, como os gerados pelo script).
pub fn parse_h264(data: &[u8]) -> Result<H264Stream> {
    let nals = split_nal_units(data);
    if nals.is_empty() {
        bail!("nenhuma NAL unit encontrada");
    }
    let has_aud = nals.iter().any(|n| nal_type(n) == NAL_AUD);

    let mut groups: Vec<Vec<&[u8]>> = Vec::new();
    let mut current: Vec<&[u8]> = Vec::new();
    let mut current_has_vcl = false;
    for nal in nals {
        let t = nal_type(nal);
        let boundary = if has_aud {
            t == NAL_AUD
        } else {
            current_has_vcl && (t == NAL_SPS || is_vcl(t))
        };
        if boundary && !current.is_empty() {
            groups.push(std::mem::take(&mut current));
            current_has_vcl = false;
        }
        if t != NAL_AUD {
            current_has_vcl |= is_vcl(t);
            current.push(nal);
        }
    }
    if !current.is_empty() {
        groups.push(current);
    }

    let mut sps: Option<Vec<u8>> = None;
    let mut pps: Option<Vec<u8>> = None;
    let mut access_units = Vec::with_capacity(groups.len());
    for group in groups {
        let keyframe = group.iter().any(|n| nal_type(n) == NAL_IDR);
        for nal in &group {
            match nal_type(nal) {
                NAL_SPS => sps = Some(nal.to_vec()),
                NAL_PPS => pps = Some(nal.to_vec()),
                _ => {}
            }
        }
        if !group.iter().any(|n| is_vcl(nal_type(n))) {
            continue;
        }
        let mut out = Vec::new();
        // Garante SPS/PPS antes de cada IDR para que novos espectadores
        // possam iniciar a decodificação em qualquer keyframe.
        if keyframe && !group.iter().any(|n| nal_type(n) == NAL_SPS) {
            let (Some(s), Some(p)) = (&sps, &pps) else {
                bail!("IDR sem SPS/PPS anteriores");
            };
            for nal in [s.as_slice(), p.as_slice()] {
                out.extend_from_slice(&START_CODE);
                out.extend_from_slice(nal);
            }
        }
        for nal in group {
            out.extend_from_slice(&START_CODE);
            out.extend_from_slice(nal);
        }
        access_units.push(AccessUnit {
            data: Bytes::from(out),
            keyframe,
        });
    }

    let sps = sps.context("fluxo sem SPS")?;
    if sps.len() < 4 {
        bail!("SPS inválido");
    }
    if !access_units.first().is_some_and(|au| au.keyframe) {
        bail!("o fluxo precisa começar com um keyframe");
    }
    Ok(H264Stream {
        access_units,
        codec: format!("avc1.{:02X}{:02X}{:02X}", sps[1], sps[2], sps[3]),
    })
}

#[derive(Debug)]
pub struct OpusStream {
    pub channels: u8,
    pub sample_rate: u32,
    /// Pacotes Opus com duração em microssegundos.
    pub packets: Vec<(Bytes, u64)>,
}

/// Extrai pacotes Opus de um arquivo Ogg (RFC 7845), ignorando OpusHead e OpusTags.
pub fn parse_ogg_opus(data: &[u8]) -> Result<OpusStream> {
    let mut packets: Vec<Vec<u8>> = Vec::new();
    let mut partial: Vec<u8> = Vec::new();
    let mut pos = 0;
    while pos < data.len() {
        let page = &data[pos..];
        if page.len() < 27 || &page[..4] != b"OggS" {
            bail!("página Ogg inválida no offset {pos}");
        }
        let segments = page[26] as usize;
        let lacing = page
            .get(27..27 + segments)
            .context("tabela de lacing truncada")?;
        let mut body = 27 + segments;
        for &len in lacing {
            let len = len as usize;
            let seg = page
                .get(body..body + len)
                .context("segmento Ogg truncado")?;
            partial.extend_from_slice(seg);
            body += len;
            if len < 255 {
                packets.push(std::mem::take(&mut partial));
            }
        }
        pos += body;
    }

    let mut iter = packets.into_iter();
    let head = iter.next().context("OpusHead ausente")?;
    if head.len() < 19 || &head[..8] != b"OpusHead" {
        bail!("OpusHead inválido");
    }
    let channels = head[9];
    let _tags = iter.next().context("OpusTags ausente")?;
    let packets = iter
        .filter(|p| !p.is_empty())
        .map(|p| {
            let dur = opus_packet_duration_us(&p);
            (Bytes::from(p), dur)
        })
        .collect::<Vec<_>>();
    if packets.is_empty() {
        bail!("nenhum pacote de áudio");
    }
    // Pacotes Opus sempre são decodificados em 48 kHz (RFC 7845 §5.1).
    Ok(OpusStream {
        channels,
        sample_rate: 48_000,
        packets,
    })
}

/// Duração de um pacote Opus a partir do byte TOC (RFC 6716 §3.1).
pub fn opus_packet_duration_us(packet: &[u8]) -> u64 {
    let Some(&toc) = packet.first() else {
        return 0;
    };
    let config = toc >> 3;
    let frame_us: u64 = match config {
        0..=11 => [10_000, 20_000, 40_000, 60_000][(config % 4) as usize],
        12..=15 => [10_000, 20_000][(config % 2) as usize],
        _ => [2_500, 5_000, 10_000, 20_000][(config % 4) as usize],
    };
    let frames = match toc & 0x3 {
        0 => 1,
        1 | 2 => 2,
        _ => packet.get(1).map_or(0, |b| u64::from(b & 0x3F)),
    };
    frame_us * frames
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nal(t: u8, extra: &[u8]) -> Vec<u8> {
        let mut v = vec![0, 0, 0, 1, 0x60 | t];
        v.extend_from_slice(extra);
        v
    }

    #[test]
    fn splits_three_and_four_byte_start_codes() {
        let data = [0, 0, 0, 1, 0x67, 1, 2, 0, 0, 1, 0x68, 3];
        let nals = split_nal_units(&data);
        assert_eq!(nals, vec![&[0x67, 1, 2][..], &[0x68, 3][..]]);
    }

    #[test]
    fn groups_by_aud_and_detects_keyframes() {
        let mut data = Vec::new();
        data.extend(nal(NAL_AUD, &[0xF0]));
        data.extend(nal(NAL_SPS, &[0x64, 0x00, 0x2A, 0xAC]));
        data.extend(nal(NAL_PPS, &[0xEE]));
        data.extend(nal(NAL_IDR, &[0x88, 1]));
        data.extend(nal(NAL_AUD, &[0xF0]));
        data.extend(nal(NAL_SLICE, &[0x9A, 2]));
        let s = parse_h264(&data).expect("parse");
        assert_eq!(s.codec, "avc1.64002A");
        assert_eq!(s.access_units.len(), 2);
        assert!(s.access_units[0].keyframe);
        assert!(!s.access_units[1].keyframe);
        // AUD é removido; SPS+PPS+IDR permanecem.
        assert_eq!(split_nal_units(&s.access_units[0].data).len(), 3);
    }

    #[test]
    fn prepends_parameter_sets_to_bare_idr() {
        let mut data = Vec::new();
        data.extend(nal(NAL_SPS, &[0x42, 0xC0, 0x1F, 0x00]));
        data.extend(nal(NAL_PPS, &[0xCE]));
        data.extend(nal(NAL_IDR, &[0x88]));
        data.extend(nal(NAL_SLICE, &[0x9A]));
        data.extend(nal(NAL_IDR, &[0x88]));
        let s = parse_h264(&data).expect("parse");
        assert_eq!(s.access_units.len(), 3);
        assert_eq!(split_nal_units(&s.access_units[2].data).len(), 3);
    }

    #[test]
    fn rejects_stream_without_keyframe_start() {
        let mut data = nal(NAL_SPS, &[0x42, 0xC0, 0x1F, 0x00]);
        data.extend(nal(NAL_SLICE, &[0x9A]));
        assert!(parse_h264(&data).is_err());
    }

    #[test]
    fn opus_durations() {
        // CELT 20 ms (config 31), 1 quadro.
        assert_eq!(opus_packet_duration_us(&[31 << 3]), 20_000);
        // SILK 20 ms (config 1), 2 quadros.
        assert_eq!(opus_packet_duration_us(&[(1 << 3) | 1]), 40_000);
        // Código 3 com 3 quadros de 10 ms (config 30).
        assert_eq!(opus_packet_duration_us(&[(30 << 3) | 3, 3]), 30_000);
    }

    fn ogg_page(packets: &[&[u8]]) -> Vec<u8> {
        let mut lacing = Vec::new();
        let mut body = Vec::new();
        for p in packets {
            let mut n = p.len();
            while n >= 255 {
                lacing.push(255);
                n -= 255;
            }
            lacing.push(n as u8);
            body.extend_from_slice(p);
        }
        let mut page = b"OggS".to_vec();
        page.extend_from_slice(&[0; 22]);
        page.push(lacing.len() as u8);
        page.extend(lacing);
        page.extend(body);
        page
    }

    #[test]
    fn parses_ogg_opus() {
        let mut head = b"OpusHead".to_vec();
        head.extend_from_slice(&[1, 2, 0x38, 0x01, 0x80, 0xBB, 0, 0, 0, 0, 0]);
        let big = vec![31 << 3; 300];
        let mut data = ogg_page(&[&head]);
        data.extend(ogg_page(&[b"OpusTags"]));
        data.extend(ogg_page(&[&[31 << 3, 1, 2], &big]));
        let s = parse_ogg_opus(&data).expect("parse");
        assert_eq!(s.channels, 2);
        assert_eq!(s.packets.len(), 2);
        assert_eq!(s.packets[1].0.len(), 300);
        assert_eq!(s.packets[0].1, 20_000);
    }
}
