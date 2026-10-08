//! Conversão BGRA → NV12 na CPU (fallback sem `ID3D11VideoProcessor`).
//!
//! Usada apenas quando o adaptador não oferece processamento de vídeo D3D11
//! (máquinas virtuais, Área de Trabalho Remota, driver de vídeo básico). O
//! caminho normal converte na GPU (`convert.rs`).
//!
//! Saída em BT.709, faixa de estúdio (16–235), igual ao caminho de GPU, com o
//! mesmo encaixe proporcional (letterbox) de [`crate::sizing::letterbox`].
//! A escala usa o vizinho mais próximo: barata e suficiente para um fallback.

use crate::sizing::letterbox;

/// Tamanho em bytes de um quadro NV12 `w`×`h` (plano Y + plano UV intercalado).
pub fn nv12_len(w: u32, h: u32) -> usize {
    let (w, h) = (w as usize, h as usize);
    w * h + w * (h / 2)
}

#[inline]
fn luma(r: i32, g: i32, b: i32) -> u8 {
    (((47 * r + 157 * g + 16 * b + 128) >> 8) + 16) as u8
}

#[inline]
fn chroma(r: i32, g: i32, b: i32) -> (u8, u8) {
    let cb = ((-26 * r - 86 * g + 112 * b + 128) >> 8) + 128;
    let cr = ((112 * r - 102 * g - 10 * b + 128) >> 8) + 128;
    (cb.clamp(16, 240) as u8, cr.clamp(16, 240) as u8)
}

/// Converte `src` (BGRA, `src_pitch` bytes por linha, `src_w`×`src_h`) para NV12
/// `out_w`×`out_h` em `dst`, preenchendo as bordas do letterbox com preto.
///
/// `out_w` e `out_h` devem ser pares; `dst` precisa ter pelo menos
/// [`nv12_len`]`(out_w, out_h)` bytes. Retorna `false` se os tamanhos não batem.
pub fn bgra_to_nv12(
    src: &[u8],
    src_pitch: usize,
    src_w: u32,
    src_h: u32,
    dst: &mut [u8],
    out_w: u32,
    out_h: u32,
) -> bool {
    let (ow, oh) = (out_w as usize, out_h as usize);
    if ow == 0 || oh == 0 || ow % 2 != 0 || oh % 2 != 0 || dst.len() < nv12_len(out_w, out_h) {
        return false;
    }
    if src_w == 0 || src_h == 0 || src_pitch < src_w as usize * 4 {
        return false;
    }
    if src.len() < src_pitch * (src_h as usize - 1) + src_w as usize * 4 {
        return false;
    }

    let (y_plane, uv_plane) = dst.split_at_mut(ow * oh);
    y_plane.fill(16);
    uv_plane[..ow * (oh / 2)].fill(128);

    let (x0, y0, w, h) = letterbox(src_w, src_h, out_w, out_h);
    let (x0, y0, w, h) = (x0 as usize, y0 as usize, w as usize, h as usize);
    let (sw, sh) = (src_w as usize, src_h as usize);

    let pixel = |dx: usize, dy: usize| -> (i32, i32, i32) {
        let sx = (dx * sw / w).min(sw - 1);
        let sy = (dy * sh / h).min(sh - 1);
        let i = sy * src_pitch + sx * 4;
        (
            i32::from(src[i + 2]),
            i32::from(src[i + 1]),
            i32::from(src[i]),
        )
    };

    // Blocos 2×2: quatro amostras de luma e uma de croma (média).
    for by in (0..h).step_by(2) {
        for bx in (0..w).step_by(2) {
            let (mut rs, mut gs, mut bs) = (0, 0, 0);
            for (ox, oy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let (dx, dy) = ((bx + ox).min(w - 1), (by + oy).min(h - 1));
                let (r, g, b) = pixel(dx, dy);
                y_plane[(y0 + by + oy) * ow + x0 + bx + ox] = luma(r, g, b);
                rs += r;
                gs += g;
                bs += b;
            }
            let (cb, cr) = chroma(rs / 4, gs / 4, bs / 4);
            let uv = ((y0 + by) / 2) * ow + x0 + bx;
            uv_plane[uv] = cb;
            uv_plane[uv + 1] = cr;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: u32, h: u32, bgra: [u8; 4]) -> Vec<u8> {
        bgra.repeat((w * h) as usize)
    }

    fn convert(src: &[u8], sw: u32, sh: u32, ow: u32, oh: u32) -> Vec<u8> {
        let mut dst = vec![0u8; nv12_len(ow, oh)];
        assert!(bgra_to_nv12(src, sw as usize * 4, sw, sh, &mut dst, ow, oh));
        dst
    }

    #[test]
    fn white_and_black_map_to_studio_range() {
        let white = convert(&solid(4, 4, [255, 255, 255, 255]), 4, 4, 4, 4);
        assert!(white[..16].iter().all(|&y| y == 235), "{:?}", &white[..16]);
        assert!(white[16..].iter().all(|&c| c == 128));
        let black = convert(&solid(4, 4, [0, 0, 0, 255]), 4, 4, 4, 4);
        assert!(black[..16].iter().all(|&y| y == 16));
        assert!(black[16..].iter().all(|&c| c == 128));
    }

    #[test]
    fn pure_red_matches_bt709() {
        // BT.709 estúdio para vermelho puro: Y≈63, Cb≈102, Cr≈240.
        let red = convert(&solid(2, 2, [0, 0, 255, 255]), 2, 2, 2, 2);
        assert!((62..=64).contains(&red[0]), "Y={}", red[0]);
        assert!((101..=103).contains(&red[4]), "Cb={}", red[4]);
        assert_eq!(red[5], 240);
    }

    #[test]
    fn letterbox_borders_are_black() {
        // Fonte 4:3 em saída 16:9 → barras laterais pretas.
        let out = convert(&solid(8, 6, [255, 255, 255, 255]), 8, 6, 16, 6);
        let row: Vec<u8> = out[..16].to_vec();
        assert_eq!(row[0], 16, "borda esquerda");
        assert_eq!(row[15], 16, "borda direita");
        assert_eq!(row[8], 235, "conteúdo central");
    }

    #[test]
    fn downscales_and_respects_pitch() {
        // Fonte 8×4 com padding de 8 bytes por linha, reduzida para 4×2.
        let (sw, sh, pitch) = (8u32, 4u32, 8 * 4 + 8);
        let mut src = vec![0u8; pitch * sh as usize];
        for y in 0..sh as usize {
            for x in 0..sw as usize {
                let v = if x < 4 { 0 } else { 255 };
                src[y * pitch + x * 4..y * pitch + x * 4 + 4].copy_from_slice(&[v, v, v, 255]);
            }
        }
        let mut dst = vec![0u8; nv12_len(4, 2)];
        assert!(bgra_to_nv12(&src, pitch, sw, sh, &mut dst, 4, 2));
        assert_eq!(&dst[..4], &[16, 16, 235, 235]);
    }

    #[test]
    fn rejects_invalid_sizes() {
        let src = solid(4, 4, [0, 0, 0, 255]);
        let mut small = vec![0u8; 10];
        assert!(!bgra_to_nv12(&src, 16, 4, 4, &mut small, 4, 4));
        let mut dst = vec![0u8; nv12_len(4, 4)];
        assert!(
            !bgra_to_nv12(&src, 16, 4, 4, &mut dst, 3, 4),
            "largura ímpar"
        );
        assert!(
            !bgra_to_nv12(&src, 8, 4, 4, &mut dst, 4, 4),
            "pitch menor que a linha"
        );
        assert!(
            !bgra_to_nv12(&src[..20], 16, 4, 4, &mut dst, 4, 4),
            "fonte truncada"
        );
    }
}
