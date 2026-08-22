//! 素材转换：MP4 → 项目可用的 VP9-alpha-webm（作为内置能力供异步转换作业调用）。
//!
//! 对齐《素材转换与集成方案》§2：
//! 1. HSV 色相绿幕抠像（仅绿色相 70~170° 且 饱和/明度≥0.15 判为背景，非绿色不误抠）；
//! 2. despill 去绿边（前景/边缘绿色过量压回 max(R,B)）；
//! 3. 水印：四角边距整块置透明（Doubao 水印落四角）；
//! 4. 归一化到 640×360（每段独立：字符 alpha 包围盒 → 等比缩放到目标高 → 居中、脚底对齐；
//!    跨动画共享基准由调用方/批量工具负责，这里保持单段归一化）；
//! 5. 编码：`-c:v libvpx-vp9 -pix_fmt yuva420p` 写 VP9-alpha-webm（alpha 存 BlockAdditional）。
//!
//! 解封装/解码/编码/scale/alpha-mux 依赖系统 **ffmpeg / ffprobe** 二进制（约定见 §4，本迭代决策）。
//! Rust 负责 HSV 抠像 / despill / 水印 / 归一化（与 `src/bin/convert_asset.rs` 同一套逻辑）。
#![allow(dead_code)]

use std::io::{Read, Write};
use std::process::{Command, Stdio};

/// 转换参数（默认对齐 convert_asset）。
#[derive(Clone, Debug)]
pub struct ConvertOptions {
    pub hue_min: f32,
    pub hue_max: f32,
    pub sat_min: f32,
    pub val_min: f32,
    pub feather: f32,
    pub despill: f32,
    pub watermark: bool,
    pub canvas_w: usize,
    pub canvas_h: usize,
    pub target_h: f64,
    pub foot_margin: i64,
    pub crf: i32,
    pub normalize: bool,
}

impl Default for ConvertOptions {
    fn default() -> Self {
        ConvertOptions {
            hue_min: 70.0,
            hue_max: 170.0,
            sat_min: 0.15,
            val_min: 0.15,
            feather: 6.0,
            despill: 0.9,
            watermark: true,
            canvas_w: 640,
            canvas_h: 360,
            target_h: 295.0,
            foot_margin: 30,
            crf: 30,
            normalize: true,
        }
    }
}

struct P {
    lo: f32,
    hi: f32,
    sat_min: f32,
    val_min: f32,
    feather: f32,
    despill: f32,
    watermark: bool,
    mw: usize,
    mh: usize,
}

impl P {
    fn from(opts: &ConvertOptions, w: usize, h: usize) -> P {
        P {
            lo: opts.hue_min - opts.feather,
            hi: opts.hue_max + opts.feather,
            sat_min: opts.sat_min,
            val_min: opts.val_min,
            feather: opts.feather,
            despill: opts.despill,
            watermark: opts.watermark,
            mw: w * 24 / 100,
            mh: h * 15 / 100,
        }
    }
}

/// 解析 ffmpeg 可执行路径。GUI 应用（`.app` 从 Finder/Dock 启动）的 PATH **不含**
/// Homebrew 目录，不能靠裸命令名找到，必须给绝对路径。优先级：
/// `DESKPET_FFMPEG` 环境变量 → `/opt/homebrew/bin`（Apple Silicon）→ `/usr/local/bin`（Intel）
/// → `/usr/bin` → 回退裸命令名 `ffmpeg`（依赖 PATH）。结果缓存（进程内一次）。
fn ffmpeg_path() -> String {
    static P: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    P.get_or_init(|| {
        if let Ok(v) = std::env::var("DESKPET_FFMPEG") {
            if !v.trim().is_empty() {
                return v;
            }
        }
        for d in ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"] {
            let f = format!("{}/ffmpeg", d);
            if std::path::Path::new(&f).is_file() {
                return f;
            }
        }
        "ffmpeg".to_string()
    })
    .clone()
}

/// 解析 ffprobe 可执行路径。优先 `DESKPET_FFPROBE`，否则与 ffmpeg 同目录推断，再回退裸命令名。
fn ffprobe_path() -> String {
    static P: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    P.get_or_init(|| {
        if let Ok(v) = std::env::var("DESKPET_FFPROBE") {
            if !v.trim().is_empty() {
                return v;
            }
        }
        let ff = ffmpeg_path();
        if ff != "ffmpeg" {
            if let Some((dir, _)) = ff.rsplit_once('/') {
                return format!("{}/ffprobe", dir);
            }
        }
        "ffprobe".to_string()
    })
    .clone()
}

/// 转换单段 mp4 → webm。`progress` 回调收到 (0..1 进度, 消息)，用于作业进度上报。
/// 成功返回输出尺寸 (w, h)。
pub fn convert_file(
    src: &str,
    dst: &str,
    opts: &ConvertOptions,
    progress: &mut dyn FnMut(f64, &str),
) -> Result<(usize, usize), String> {
    let (w, h, fps, duration) = probe(src).map_err(|e| e)?;
    progress(0.0, &format!("输入 {}x{} @ {:.2}fps", w, h, fps));
    let p = P::from(opts, w, h);

    // 归一化：第一遍算字符 alpha 包围盒，第二遍编码。
    let crop = if opts.normalize {
        match compute_union_bbox(src, w, h, &p) {
            None => {
                progress(0.05, "未检测到角色（全透明），关闭归一化输出原尺寸");
                None
            }
            Some((x0, y0, x1, y1)) => {
                let x0 = x0.saturating_sub(6);
                let y0 = y0.saturating_sub(6);
                let x1 = (x1 + 6).min(w);
                let y1 = (y1 + 6).min(h);
                let cw = x1 - x0;
                let ch = y1 - y0;
                let scale_f = (opts.target_h / ch as f64) as f32;
                let mut sw = (cw as f32 * scale_f).round() as i64;
                let sh = opts.target_h as i64;
                if sw % 2 != 0 {
                    sw += 1;
                }
                let sw = sw.max(2) as usize;
                let sh = sh.max(2) as usize;
                let dx = ((opts.canvas_w as i64 - sw as i64) / 2).max(0).min((opts.canvas_w - sw) as i64) as usize;
                let dy = (opts.canvas_h as i64 - opts.foot_margin - sh as i64).max(0).min((opts.canvas_h - sh) as i64) as usize;
                progress(0.1, &format!("归一化: 裁剪 {x0},{y0} {cw}x{ch} → 缩放 {sw}x{sh} → 画布 {}x{} @ ({dx},{dy})", opts.canvas_w, opts.canvas_h));
                Some(format!(
                    "crop={}:{}:{}:{},scale={}:{},pad={}:{}:{}:{}:0x00000000",
                    cw, ch, x0, y0, sw, sh, opts.canvas_w, opts.canvas_h, dx, dy
                ))
            }
        }
    } else {
        None
    };

    let vf = match &crop {
        Some(c) => format!("{},format=yuva420p", c),
        None => "format=yuva420p".to_string(),
    };
    let out_w = if opts.normalize && crop.is_some() { opts.canvas_w } else { w };
    let out_h = if opts.normalize && crop.is_some() { opts.canvas_h } else { h };

    let mut dec = Command::new(ffmpeg_path())
        .args(["-loglevel", "error", "-i", src, "-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|e| format!("启动 ffmpeg 解码失败（{}）: {}", ffmpeg_path(), e))?;

    let mut enc = Command::new(ffmpeg_path())
        .args([
            "-y", "-loglevel", "error",
            "-f", "rawvideo", "-pix_fmt", "rgba", "-s", &format!("{}x{}", w, h),
            "-r", &format!("{:.3}", fps), "-i", "-",
            "-vf", &vf,
            "-c:v", "libvpx-vp9", "-pix_fmt", "yuva420p",
            "-crf", &opts.crf.to_string(), "-b:v", "0", "-row-mt", "1", "-threads", "0",
            "-an", dst,
        ])
        .stdin(Stdio::piped())
        .spawn()
        .map_err(|e| format!("启动 ffmpeg 编码失败: {}", e))?;

    let rgb_frame = w * h * 3;
    let rgba_frame = w * h * 4;
    let mut dec_out = dec.stdout.take().unwrap();
    let mut enc_in = enc.stdin.take().unwrap();
    let mut buf = vec![0u8; rgb_frame];
    let mut rgba = vec![0u8; rgba_frame];
    let mut frame_idx = 0usize;
    let total_frames = (duration * fps).max(1.0);
    let start = std::time::Instant::now();

    loop {
        match dec_out.read_exact(&mut buf) {
            Ok(_) => {}
            Err(_) => break,
        }
        key_frame(&buf, &mut rgba, w, h, &p);
        if let Err(e) = enc_in.write_all(&rgba) {
            return Err(format!("写编码器失败: {}", e));
        }
        frame_idx += 1;
        if frame_idx % 4 == 0 || frame_idx == 1 {
            let prog = ((frame_idx as f64) / total_frames).min(1.0);
            progress(prog, &format!("已处理 {} 帧", frame_idx));
        }
    }
    drop(dec_out);
    drop(enc_in);
    let _ = dec.wait();
    let encrc = enc.wait().map_err(|e| format!("等待编码器失败: {}", e))?;

    if frame_idx == 0 {
        return Err("解码到 0 帧".to_string());
    }
    if !encrc.success() {
        return Err(format!("编码器非零退出 ({})", encrc.code().unwrap_or(-1)));
    }
    progress(1.0, &format!("完成 -> {} ({}x{})，耗时 {:.1}s", dst, out_w, out_h, start.elapsed().as_secs_f32()));
    Ok((out_w, out_h))
}

/// 用 ffprobe 读 width/height/fps/duration。
fn probe(src: &str) -> Result<(usize, usize, f64, f64), String> {
    let out = Command::new(ffprobe_path())
        .args([
            "-v", "error", "-select_streams", "v:0",
            "-show_entries", "stream=width,height,avg_frame_rate",
            "-of", "default=nw=1:nk=1", src,
        ])
        .output()
        .map_err(|e| {
            format!(
                "无法启动 ffprobe（{}）：{}。请安装 ffmpeg，或用环境变量 DESKPET_FFMPEG/DESKPET_FFPROBE 指定其路径",
                ffprobe_path(),
                e
            )
        })?;
    let text = String::from_utf8_lossy(&out.stdout);
    let mut w = 1280usize;
    let mut h = 720usize;
    let mut fps = 24.0f64;
    for (i, line) in text.lines().enumerate() {
        match i {
            0 => w = line.trim().parse().unwrap_or(1280),
            1 => h = line.trim().parse().unwrap_or(720),
            _ if line.contains('/') => {
                let mut pp = line.trim().split('/');
                let num: f64 = pp.next().and_then(|s| s.parse().ok()).unwrap_or(24.0);
                let den: f64 = pp.next().and_then(|s| s.parse().ok()).unwrap_or(1.0);
                fps = if den != 0.0 { num / den } else { 24.0 };
            }
            _ => {}
        }
    }
    let dur = Command::new(ffprobe_path())
        .args(["-v", "error", "-show_entries", "format=duration", "-of", "default=nw=1:nk=1", src])
        .output()
        .map_err(|e| format!("调用 ffprobe 读取时长失败: {}", e))?;
    let dur_text = String::from_utf8_lossy(&dur.stdout);
    let duration = dur_text.trim().parse::<f64>().unwrap_or(0.0);
    Ok((w, h, fps, duration))
}

/// 计算字符 alpha 包围盒（跨帧并集，采样每 3 帧取 1）。
fn compute_union_bbox(src: &str, w: usize, h: usize, p: &P) -> Option<(usize, usize, usize, usize)> {
    let mut dec = Command::new(ffmpeg_path())
        .args(["-loglevel", "error", "-i", src, "-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
        .stdout(Stdio::piped())
        .spawn()
        .ok()?;
    let frame = w * h * 3;
    let mut out = dec.stdout.take()?;
    let mut buf = vec![0u8; frame];
    let mut bbox: Option<(usize, usize, usize, usize)> = None;
    let mut n = 0usize;
    loop {
        match out.read_exact(&mut buf) {
            Ok(_) => {}
            Err(_) => break,
        }
        if n % 3 == 0 {
            let mut alpha = vec![0u8; w * h];
            alpha_of_frame(&buf, &mut alpha, w, h, p);
            for y in 0..h {
                let row = y * w;
                for x in 0..w {
                    if alpha[row + x] > 16 {
                        if let Some((x0, y0, x1, y1)) = bbox.as_mut() {
                            if x < *x0 { *x0 = x; }
                            if y < *y0 { *y0 = y; }
                            if x > *x1 { *x1 = x; }
                            if y > *y1 { *y1 = y; }
                        } else {
                            bbox = Some((x, y, x, y));
                        }
                    }
                }
            }
        }
        n += 1;
    }
    let _ = dec.wait();
    bbox
}

/// 单帧 RGB → RGBA（straight alpha）。
fn key_frame(rgb: &[u8], out_rgba: &mut [u8], w: usize, h: usize, p: &P) {
    for y in 0..h {
        let row = y * w;
        for x in 0..w {
            let i = (row + x) * 3;
            let (r, g, b) = (rgb[i] as f32, rgb[i + 1] as f32, rgb[i + 2] as f32);
            let (rr, gg, bb, a) = key_px(r, g, b, x, y, w, h, p);
            let o = (row + x) * 4;
            out_rgba[o] = rr as u8;
            out_rgba[o + 1] = gg as u8;
            out_rgba[o + 2] = bb as u8;
            out_rgba[o + 3] = a as u8;
        }
    }
}

/// 计算 alpha 平面（供包围盒）。
fn alpha_of_frame(rgb: &[u8], out_alpha: &mut [u8], w: usize, h: usize, p: &P) {
    for y in 0..h {
        let row = y * w;
        for x in 0..w {
            let i = (row + x) * 3;
            let (r, g, b) = (rgb[i] as f32, rgb[i + 1] as f32, rgb[i + 2] as f32);
            let (_, _, _, a) = key_px(r, g, b, x, y, w, h, p);
            out_alpha[row + x] = a as u8;
        }
    }
}

/// 单像素 HSV 抠像 + 水印 + despill。
#[inline]
fn key_px(r: f32, g: f32, b: f32, x: usize, y: usize, w: usize, h: usize, p: &P) -> (f32, f32, f32, f32) {
    let (hue, sat, val) = rgb_to_hsv(r, g, b);
    let in_hue = clamp((hue - p.lo) / p.feather, 0.0, 1.0) * clamp((p.hi - hue) / p.feather, 0.0, 1.0);
    let bg = sat >= p.sat_min && val >= p.val_min && hue >= p.lo && hue <= p.hi;
    let mut a = if bg { (1.0 - in_hue) * 255.0 } else { 255.0 };
    if p.watermark && ((x < p.mw || x >= w - p.mw) && (y < p.mh || y >= h - p.mh)) {
        a = 0.0;
    }
    let rr = r;
    let mut gg = g;
    let bb = b;
    if a > 0.0 {
        let m = rr.max(bb);
        if gg > m {
            let nv = gg - (gg - m) * p.despill;
            if nv >= 0.0 { gg = nv; }
        }
    }
    (rr, gg, bb, a.round().clamp(0.0, 255.0))
}

#[inline]
fn rgb_to_hsv(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    let rn = r / 255.0;
    let gn = g / 255.0;
    let bn = b / 255.0;
    let mx = rn.max(gn).max(bn);
    let mn = rn.min(gn).min(bn);
    let delta = mx - mn;
    if delta <= 0.0 { return (0.0, 0.0, mx); }
    let hue = if mx == rn {
        60.0 * (((gn - bn) / delta) % 6.0)
    } else if mx == gn {
        60.0 * (((bn - rn) / delta) + 2.0)
    } else {
        60.0 * (((rn - gn) / delta) + 4.0)
    };
    let hue = if hue < 0.0 { hue + 360.0 } else { hue };
    let sat = if mx > 0.0 { delta / mx } else { 0.0 };
    (hue, sat, mx)
}

#[inline]
fn clamp(x: f32, lo: f32, hi: f32) -> f32 {
    if x < lo { lo } else if x > hi { hi } else { x }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p() -> P {
        let opts = ConvertOptions::default();
        P::from(&opts, 640, 360)
    }

    #[test]
    fn rgb_to_hsv_basic() {
        let (h, s, v) = rgb_to_hsv(255.0, 0.0, 0.0);
        assert!((h - 0.0).abs() < 0.01);
        assert!((s - 1.0).abs() < 0.01);
        assert!((v - 1.0).abs() < 0.01);
        let (h, _, _) = rgb_to_hsv(0.0, 255.0, 0.0);
        assert!((h - 120.0).abs() < 0.01);
    }

    #[test]
    fn green_is_background_blue_is_foreground() {
        let pp = p();
        // 纯绿 → 背景（alpha 接近 0）
        let (_, _, _, a) = key_px(0.0, 255.0, 0.0, 100, 100, 640, 360, &pp);
        assert!(a < 10.0, "green alpha = {}", a);
        // 纯蓝 → 前景（alpha 255，无 despill）
        let (_, g, _, a) = key_px(0.0, 0.0, 255.0, 100, 100, 640, 360, &pp);
        assert_eq!(a, 255.0);
        assert_eq!(g, 0.0);
    }

    #[test]
    fn clamp_bounds() {
        assert_eq!(clamp(5.0, 0.0, 1.0), 1.0);
        assert_eq!(clamp(-3.0, 0.0, 1.0), 0.0);
        assert_eq!(clamp(0.5, 0.0, 1.0), 0.5);
    }
}
