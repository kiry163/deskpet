//! 帧提取：从 webm 解码一帧并编码为 **PNG**（全身照 / 缩略图预览用）。
//!
//! 设计要点：
//! - 选帧策略：统计每帧「有效（alpha>8）像素数」，取最多的一帧 —— 即最完整的身体姿态
//!   （呼吸/待机动画的最高点通常轮廓最饱满），作为全身照；
//! - 输出**直通 RGBA**（非预乘 alpha）PNG，透明背景与角色边缘保持干净，无暗边；
//! - 纯函数、无 GUI 依赖：供控制台/导入流程在后台线程调用。

use crate::clip;
use crate::vpx::Decoder;
use crate::webm::WebM;

/// 从完整 webm 字节提取一帧 PNG。
pub fn extract_png(webm_bytes: &[u8]) -> Option<Vec<u8>> {
    let wm = WebM::parse(webm_bytes)?;
    extract_png_webm(&wm)
}

/// 从已解析的 WebM 提取一帧 PNG（选「有效像素最多」的帧）。
pub fn extract_png_webm(wm: &WebM) -> Option<Vec<u8>> {
    let n = wm.frames.len();
    if n == 0 {
        return None;
    }
    let mut color_dec = Decoder::new(1)?;
    let mut alpha_dec = Decoder::new(1)?;
    let ytab = clip::make_ytab();

    let mut best_opaque = 0u64;
    let mut best_rgba: Vec<u8> = Vec::new();
    let mut rgba: Vec<u8> = Vec::new();
    let mut bw = 0usize;
    let mut bh = 0usize;

    for i in 0..n {
        let f = &wm.frames[i];
        let color = match color_dec.decode(&f.video) {
            Some(c) => c,
            None => continue,
        };
        let alpha = match (&f.alpha, &mut alpha_dec) {
            (Some(a), ad) => ad.decode(a),
            _ => None,
        };
        let (cw, ch) = clip::compose_rgba_into(color, alpha, &ytab, &mut rgba);
        let opaque = count_opaque(&rgba);
        if opaque > best_opaque {
            best_opaque = opaque;
            bw = cw;
            bh = ch;
            best_rgba.clear();
            best_rgba.extend_from_slice(&rgba);
        }
    }

    // 兜底：全帧透明（异常素材）→ 强取第一帧
    if best_opaque == 0 {
        let f = &wm.frames[0];
        let color = color_dec.decode(&f.video)?;
        let alpha = match (&f.alpha, &mut alpha_dec) {
            (Some(a), ad) => ad.decode(a),
            _ => None,
        };
        let (cw, ch) = clip::compose_rgba_into(color, alpha, &ytab, &mut best_rgba);
        bw = cw;
        bh = ch;
        best_opaque = count_opaque(&best_rgba);
    }
    if best_opaque == 0 {
        return None;
    }
    encode_png(&best_rgba, bw, bh)
}

/// 统计有效像素数（alpha 通道 > 8 视为有效）。
fn count_opaque(rgba: &[u8]) -> u64 {
    rgba.chunks_exact(4).filter(|p| p[3] > 8).count() as u64
}

/// 编码 RGBA8 为 PNG 字节。
fn encode_png(rgba: &[u8], w: usize, h: usize) -> Option<Vec<u8>> {
    let mut buf = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut buf, w as u32, h as u32);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header().ok()?;
        writer.write_image_data(rgba).ok()?;
    }
    Some(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn count_opaque_counts_alpha_above_threshold() {
        // 8 像素：3 个有效（alpha>8），其余无效
        let rgba: Vec<u8> = [
            1, 2, 3, 255, // valid
            1, 2, 3, 0,   // invalid
            1, 2, 3, 9,   // valid
            1, 2, 3, 8,   // invalid (= threshold)
            1, 2, 3, 129, // valid
            1, 2, 3, 4,   // invalid
            1, 2, 3, 255, // valid
            1, 2, 3, 0,   // invalid
        ]
        .to_vec();
        assert_eq!(count_opaque(&rgba), 4);
    }
}
