//! Cálculo de dimensões e parâmetros do encoder (puro, sem dependência de plataforma).

/// Dimensões de saída que cabem em `max_w`×`max_h` mantendo a proporção da fonte.
///
/// Nunca amplia a fonte; resultado sempre par (exigência do NV12/H.264) e ≥ 64.
pub fn fit_dimensions(src_w: u32, src_h: u32, max_w: u32, max_h: u32) -> (u32, u32) {
    if src_w == 0 || src_h == 0 {
        return (even(max_w.max(64)), even(max_h.max(64)));
    }
    let scale = f64::min(
        1.0,
        f64::min(
            f64::from(max_w) / f64::from(src_w),
            f64::from(max_h) / f64::from(src_h),
        ),
    );
    let w = (f64::from(src_w) * scale).round() as u32;
    let h = (f64::from(src_h) * scale).round() as u32;
    (even(w.max(64)), even(h.max(64)))
}

fn even(v: u32) -> u32 {
    v & !1
}

/// Retângulo de destino (x, y, w, h) que encaixa a fonte no quadro de saída
/// mantendo a proporção (letterbox), usado quando a janela muda de tamanho
/// durante a transmissão sem reiniciar o encoder.
pub fn letterbox(src_w: u32, src_h: u32, dst_w: u32, dst_h: u32) -> (u32, u32, u32, u32) {
    if src_w == 0 || src_h == 0 || dst_w == 0 || dst_h == 0 {
        return (0, 0, dst_w, dst_h);
    }
    let scale = f64::min(
        f64::from(dst_w) / f64::from(src_w),
        f64::from(dst_h) / f64::from(src_h),
    );
    let w = even(((f64::from(src_w) * scale).round() as u32).clamp(2, dst_w));
    let h = even(((f64::from(src_h) * scale).round() as u32).clamp(2, dst_h));
    let x = even((dst_w - w) / 2);
    let y = even((dst_h - h) / 2);
    (x, y, w, h)
}

/// Tamanho do GOP em quadros para um intervalo de keyframe em segundos.
pub fn gop_frames(fps: u32, keyframe_interval_s: u32) -> u32 {
    fps.max(1).saturating_mul(keyframe_interval_s.max(1))
}

/// Bitrate em bits/s a partir de kbps.
pub fn bitrate_bps(kbps: u32) -> u32 {
    kbps.saturating_mul(1000)
}

/// Duração de um quadro em unidades de 100 ns (base de tempo do Media Foundation).
pub fn frame_duration_hns(fps: u32) -> i64 {
    10_000_000 / i64::from(fps.max(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fits_without_upscaling() {
        assert_eq!(fit_dimensions(2560, 1440, 1920, 1080), (1920, 1080));
        assert_eq!(fit_dimensions(1280, 720, 1920, 1080), (1280, 720));
        assert_eq!(fit_dimensions(3440, 1440, 1920, 1080), (1920, 804));
        assert_eq!(fit_dimensions(1001, 777, 1920, 1080), (1000, 776));
    }

    #[test]
    fn fit_handles_degenerate_sources() {
        assert_eq!(fit_dimensions(0, 0, 1920, 1080), (1920, 1080));
        assert_eq!(fit_dimensions(10, 10, 1920, 1080), (64, 64));
    }

    #[test]
    fn letterbox_centers_content() {
        assert_eq!(letterbox(1920, 1080, 1920, 1080), (0, 0, 1920, 1080));
        // Janela 4:3 em saída 16:9: barras laterais.
        assert_eq!(letterbox(1024, 768, 1920, 1080), (240, 0, 1440, 1080));
        // Janela ultrawide em saída 16:9: barras em cima e embaixo.
        let (x, y, w, h) = letterbox(3440, 1440, 1920, 1080);
        assert_eq!((x, w), (0, 1920));
        assert!(y > 0 && h < 1080 && y % 2 == 0 && h % 2 == 0);
    }

    #[test]
    fn gop_and_bitrate_math() {
        assert_eq!(gop_frames(60, 2), 120);
        assert_eq!(gop_frames(0, 0), 1);
        assert_eq!(bitrate_bps(8000), 8_000_000);
        assert_eq!(frame_duration_hns(60), 166_666);
    }
}
