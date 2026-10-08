//! Utilitários puros para fluxos H.264 em Annex B.

/// Start code de 4 bytes usado na saída.
pub const START_CODE: [u8; 4] = [0, 0, 0, 1];

pub const NAL_SLICE: u8 = 1;
pub const NAL_IDR: u8 = 5;
pub const NAL_SPS: u8 = 7;
pub const NAL_PPS: u8 = 8;
pub const NAL_AUD: u8 = 9;

/// Divide um buffer Annex B em NAL units (sem start codes).
pub fn nal_units(data: &[u8]) -> Vec<&[u8]> {
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
        while end > start && data[end - 1] == 0 {
            end -= 1;
        }
        if end > start {
            nals.push(&data[start..end]);
        }
    }
    nals
}

pub fn nal_type(nal: &[u8]) -> u8 {
    nal.first().map_or(0, |b| b & 0x1F)
}

/// Parâmetros SPS/PPS mais recentes vistos no fluxo.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParameterSets {
    pub sps: Option<Vec<u8>>,
    pub pps: Option<Vec<u8>>,
}

impl ParameterSets {
    /// Atualiza a partir de um buffer Annex B (ex.: `MF_MT_MPEG_SEQUENCE_HEADER` ou um quadro).
    pub fn update_from(&mut self, data: &[u8]) -> bool {
        let mut changed = false;
        for nal in nal_units(data) {
            let slot = match nal_type(nal) {
                NAL_SPS => &mut self.sps,
                NAL_PPS => &mut self.pps,
                _ => continue,
            };
            if slot.as_deref() != Some(nal) {
                *slot = Some(nal.to_vec());
                changed = true;
            }
        }
        changed
    }

    pub fn is_complete(&self) -> bool {
        self.sps.is_some() && self.pps.is_some()
    }

    /// Codec string WebCodecs (`avc1.PPCCLL`) a partir do SPS.
    pub fn codec_string(&self) -> Option<String> {
        codec_string(self.sps.as_deref()?)
    }
}

/// `avc1.PPCCLL` a partir de um SPS (bytes após o cabeçalho NAL: profile, constraints, level).
pub fn codec_string(sps: &[u8]) -> Option<String> {
    if sps.len() < 4 || nal_type(sps) != NAL_SPS {
        return None;
    }
    Some(format!("avc1.{:02X}{:02X}{:02X}", sps[1], sps[2], sps[3]))
}

/// Unidade de acesso normalizada para o protocolo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessUnit {
    pub data: Vec<u8>,
    pub keyframe: bool,
}

/// Normaliza a saída do encoder para o formato do protocolo:
/// start codes de 4 bytes, sem AUD e com SPS/PPS antes de todo IDR.
///
/// Retorna `None` se o buffer não contém nenhuma slice (ex.: só parâmetros).
pub fn normalize(data: &[u8], params: &mut ParameterSets) -> Option<AccessUnit> {
    params.update_from(data);
    let nals = nal_units(data);
    let keyframe = nals.iter().any(|n| nal_type(n) == NAL_IDR);
    let has_vcl = nals
        .iter()
        .any(|n| (NAL_SLICE..=NAL_IDR).contains(&nal_type(n)));
    if !has_vcl {
        return None;
    }
    let has_sps = nals.iter().any(|n| nal_type(n) == NAL_SPS);
    let has_pps = nals.iter().any(|n| nal_type(n) == NAL_PPS);
    let mut out = Vec::with_capacity(data.len() + 64);
    if keyframe && !(has_sps && has_pps) {
        for nal in [params.sps.as_deref(), params.pps.as_deref()]
            .into_iter()
            .flatten()
        {
            out.extend_from_slice(&START_CODE);
            out.extend_from_slice(nal);
        }
    }
    for nal in nals {
        if nal_type(nal) == NAL_AUD {
            continue;
        }
        out.extend_from_slice(&START_CODE);
        out.extend_from_slice(nal);
    }
    Some(AccessUnit {
        data: out,
        keyframe,
    })
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
    fn splits_mixed_start_codes() {
        let data = [0, 0, 0, 1, 0x67, 1, 2, 0, 0, 1, 0x68, 3];
        assert_eq!(nal_units(&data), vec![&[0x67, 1, 2][..], &[0x68, 3][..]]);
    }

    #[test]
    fn codec_string_from_sps() {
        assert_eq!(
            codec_string(&[0x67, 0x64, 0x00, 0x2A, 0xAC]).as_deref(),
            Some("avc1.64002A")
        );
        assert_eq!(codec_string(&[0x68, 0x64, 0x00, 0x2A]), None);
        assert_eq!(codec_string(&[0x67, 0x64]), None);
    }

    #[test]
    fn prepends_cached_parameter_sets_to_bare_idr() {
        let mut params = ParameterSets::default();
        let mut header = nal(NAL_SPS, &[0x4D, 0x40, 0x28, 0x01]);
        header.extend(nal(NAL_PPS, &[0xEE]));
        assert!(params.update_from(&header));
        assert_eq!(params.codec_string().as_deref(), Some("avc1.4D4028"));

        let mut idr = nal(NAL_AUD, &[0xF0]);
        idr.extend(nal(NAL_IDR, &[0x88, 0x84]));
        let au = normalize(&idr, &mut params).expect("au");
        assert!(au.keyframe);
        let types: Vec<u8> = nal_units(&au.data).iter().map(|n| nal_type(n)).collect();
        assert_eq!(types, vec![NAL_SPS, NAL_PPS, NAL_IDR]);
        assert!(au.data.starts_with(&START_CODE));
    }

    #[test]
    fn keeps_inline_parameter_sets_and_drops_aud() {
        let mut params = ParameterSets::default();
        let mut data = nal(NAL_AUD, &[0xF0]);
        data.extend(nal(NAL_SPS, &[0x64, 0x00, 0x28, 0xAC]));
        data.extend(nal(NAL_PPS, &[0xEE]));
        data.extend(nal(NAL_IDR, &[0x88]));
        let au = normalize(&data, &mut params).expect("au");
        let types: Vec<u8> = nal_units(&au.data).iter().map(|n| nal_type(n)).collect();
        assert_eq!(types, vec![NAL_SPS, NAL_PPS, NAL_IDR]);
        assert!(params.is_complete());
    }

    #[test]
    fn delta_frames_pass_through() {
        let mut params = ParameterSets::default();
        let au = normalize(&nal(NAL_SLICE, &[0x9A, 1]), &mut params).expect("au");
        assert!(!au.keyframe);
        assert_eq!(nal_units(&au.data).len(), 1);
    }

    #[test]
    fn parameter_only_buffers_produce_nothing() {
        let mut params = ParameterSets::default();
        let mut data = nal(NAL_SPS, &[0x64, 0x00, 0x28, 0xAC]);
        data.extend(nal(NAL_PPS, &[0xEE]));
        assert_eq!(normalize(&data, &mut params), None);
        assert!(params.is_complete());
    }
}
