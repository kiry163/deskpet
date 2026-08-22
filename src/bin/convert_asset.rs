//! convert_asset —— 素材转换工具：MP4 → 项目可用的高质量 VP9-alpha-webm。
//!
//! 对齐上游 dsh-pet 的素材处理链（chroma_step02 + normalize_step03），并针对本项目画布做归一化：
//! 1. HSV 色相绿幕抠像：仅"绿色色相 70~170° 且 饱和/明度 ≥0.15"判为背景，色相边界 6° 渐变；
//!    非绿色（蓝/白/红/头发/白衣/道具）永不误抠，不受背景亮度/暗角影响（对齐上游 chroma_step02.py）。
//! 2. despill：把前景/边缘像素的绿色溢出压回 max(R,B)，消除绿边。
//! 3. 水印：四角边距整块置透明（Doubao 水印落在角落；此类素材角色居中、水印在四角）。
//! 4. 归一化：按字符 alpha 包围盒裁剪 → 等比缩放到目标高度 → 居中、脚底对齐 → 640×360 画布。
//!    项目窗口按固定画布 `CANVAS_W×CANVAS_H=640×360` 渲染（src/state.rs），素材必须为 640×360
//!    （宽度≠640 会被 render_buf 截断，见 src/pet.rs render_current）。默认素材即 640×360。
//! 5. 编码：`-c:v libvpx-vp9` 写 VP9-alpha-webm（alpha 存于 BlockAdditional，项目可读）。
//!
//! 依赖：ffmpeg / ffprobe（仅本工具运行时用，不进最终发布二进制）。
//! 用法：cargo run --release --bin convert_asset -- <in.mp4> <out.webm> [options]
//!   --hue-min 70 --hue-max 170 --sat-min 0.15 --val-min 0.15 --feather 6
//!   --despill 0.9 --crf 30 --canvas 640x360 --target-h 295 --foot-margin 30
//!   --no-watermark --no-normalize

use std::io::{Read, Write};
use std::process::{Command, Stdio};

const D_HUE_MIN: f32 = 70.0;
const D_HUE_MAX: f32 = 170.0;
const D_SAT_MIN: f32 = 0.15;
const D_VAL_MIN: f32 = 0.15;
const D_FEATHER: f32 = 6.0;

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

fn main() {
    let mut args = std::env::args().skip(1);
    let mut hue_min = D_HUE_MIN;
    let mut hue_max = D_HUE_MAX;
    let mut sat_min = D_SAT_MIN;
    let mut val_min = D_VAL_MIN;
    let mut feather = D_FEATHER;
    let mut despill = 0.9f32;
    let mut crf = 30i32;
    let mut watermark = true;
    let mut normalize = true;
    let mut cw_canvas = 640usize;
    let mut ch_canvas = 360usize;
    let mut target_h = 295f64;
    let mut foot_margin = 30i64;
    let mut src: Option<String> = None;
    let mut dst: Option<String> = None;

    while let Some(a) = args.next() {
        match a.as_str() {
            "--hue-min" => if let Some(v) = args.next() { hue_min = v.parse().unwrap_or(hue_min); },
            "--hue-max" => if let Some(v) = args.next() { hue_max = v.parse().unwrap_or(hue_max); },
            "--sat-min" => if let Some(v) = args.next() { sat_min = v.parse().unwrap_or(sat_min); },
            "--val-min" => if let Some(v) = args.next() { val_min = v.parse().unwrap_or(val_min); },
            "--feather" => if let Some(v) = args.next() { feather = v.parse().unwrap_or(feather); },
            "--despill" => if let Some(v) = args.next() { despill = v.parse().unwrap_or(despill); },
            "--crf"     => if let Some(v) = args.next() { crf = v.parse().unwrap_or(crf); },
            "--target-h" => if let Some(v) = args.next() { target_h = v.parse().unwrap_or(target_h); },
            "--foot-margin" => if let Some(v) = args.next() { foot_margin = v.parse().unwrap_or(foot_margin); },
            "--canvas" => {
                if let Some(v) = args.next() {
                    if let Some((w, h)) = v.split_once('x') {
                        cw_canvas = w.parse().unwrap_or(640);
                        ch_canvas = h.parse().unwrap_or(360);
                    }
                }
            }
            "--no-watermark" => watermark = false,
            "--no-normalize" => normalize = false,
            _ => {
                if src.is_none() { src = Some(a); }
                else if dst.is_none() { dst = Some(a); }
                else { eprintln!("未知参数: {}（见文件头注释）", a); std::process::exit(2); }
            }
        }
    }
    let (src, dst) = match (src, dst) {
        (Some(s), Some(d)) => (s, d),
        _ => { eprintln!("用法: convert_asset <in.mp4> <out.webm> [options]"); std::process::exit(2); }
    };

    let (w, h, fps) = probe(&src);
    println!("输入: {}x{} @ {:.2}fps", w, h, fps);
    let p = P {
        lo: hue_min - feather,
        hi: hue_max + feather,
        sat_min,
        val_min,
        feather,
        despill,
        watermark,
        mw: w * 24 / 100,
        mh: h * 15 / 100,
    };

    // 归一化：两遍。第一遍算字符 alpha 包围盒（稳定裁剪区），第二遍编码。
    let crop = if normalize {
        let bbox = compute_union_bbox(&src, w, h, &p);
        if bbox.is_none() {
            eprintln!("[警告] 未检测到角色（全透明），关闭归一化输出原尺寸");
            None
        } else {
            let (x0, y0, x1, y1) = bbox.unwrap();
            // 留边，避免裁剪裁到角色边缘
            let x0 = x0.saturating_sub(6);
            let y0 = y0.saturating_sub(6);
            let x1 = (x1 + 6).min(w);
            let y1 = (y1 + 6).min(h);
            let cw = x1 - x0;
            let ch = y1 - y0;
            let scale_f = (target_h / ch as f64) as f32;
            let mut sw = (cw as f32 * scale_f).round() as i64;
            let sh = target_h as i64;
            if sw % 2 != 0 { sw += 1; } // 4:2:0 需偶数宽
            let sw = sw.max(2) as usize;
            let sh = sh.max(2) as usize;
            let dx = ((cw_canvas as i64 - sw as i64) / 2).max(0).min((cw_canvas - sw) as i64) as usize;
            let dy = (ch_canvas as i64 - foot_margin - sh as i64).max(0).min((ch_canvas - sh) as i64) as usize;
            println!("归一化: 裁剪 {x0},{y0} {cw}x{ch} → 缩放 {sw}x{sh} → 画布 {cw_canvas}x{ch_canvas} @ ({dx},{dy})");
            Some(format!("crop={}:{}:{}:{},scale={}:{},pad={}:{}:{}:{}:0x00000000",
                    cw, ch, x0, y0, sw, sh, cw_canvas, ch_canvas, dx, dy))
        }
    } else {
        None
    };

    // 编码 fftp 参数：输入 rgba（含 alpha），crop/scale/pad 归一化后转 yuva420p。
    let vf = match &crop {
        Some(c) => format!("{},format=yuva420p", c),
        None => "format=yuva420p".to_string(),
    };
    let out_w = if normalize && crop.is_some() { cw_canvas } else { w };
    let out_h = if normalize && crop.is_some() { ch_canvas } else { h };

    let mut dec = Command::new("ffmpeg")
        .args(["-loglevel", "error", "-i", &src, "-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
        .stdout(Stdio::piped())
        .spawn()
        .expect("启动 ffmpeg 解码失败");

    let mut enc = Command::new("ffmpeg")
        .args([
            "-y", "-loglevel", "error",
            "-f", "rawvideo", "-pix_fmt", "rgba", "-s", &format!("{}x{}", w, h),
            "-r", &format!("{:.3}", fps), "-i", "-",
            "-vf", &vf,
            "-c:v", "libvpx-vp9", "-pix_fmt", "yuva420p",
            "-crf", &crf.to_string(), "-b:v", "0", "-row-mt", "1", "-threads", "0",
            "-an", &dst,
        ])
        .stdin(Stdio::piped())
        .spawn()
        .expect("启动 ffmpeg 编码失败");

    let rgb_frame = w * h * 3;
    let rgba_frame = w * h * 4;
    let mut dec_out = dec.stdout.take().unwrap();
    let mut enc_in = enc.stdin.take().unwrap();
    let mut buf = vec![0u8; rgb_frame];
    let mut rgba = vec![0u8; rgba_frame];
    let mut frame_idx = 0usize;
    let start = std::time::Instant::now();

    loop {
        match dec_out.read_exact(&mut buf) {
            Ok(_) => {}
            Err(_) => break,
        }
        key_frame(&buf, &mut rgba, w, h, &p);
        if let Err(e) = enc_in.write_all(&rgba) {
            eprintln!("写编码器失败: {}", e);
            break;
        }
        frame_idx += 1;
        if frame_idx % 24 == 0 || frame_idx == 1 {
            eprintln!("  已处理 {} 帧", frame_idx);
        }
    }
    drop(dec_out);
    drop(enc_in);
    let _ = dec.wait();
    let encrc = enc.wait().unwrap();
    eprintln!("共处理 {} 帧，耗时 {:.1}s", frame_idx, start.elapsed().as_secs_f32());
    if frame_idx == 0 { eprintln!("\n[失败] 解码到 0 帧"); std::process::exit(1); }
    if !encrc.success() {
        eprintln!("[警告] 编码器非零退出 ({})", encrc.code().unwrap_or(-1));
    }
    println!("\n[完成] -> {} ({out_w}x{out_h})", dst);
}

fn probe(src: &str) -> (usize, usize, f64) {
    let out = Command::new("ffprobe")
        .args(["-v", "error", "-select_streams", "v:0",
               "-show_entries", "stream=width,height,avg_frame_rate",
               "-of", "default=nw=1:nk=1", src])
        .output()
        .expect("调用 ffprobe 失败");
    let text = String::from_utf8_lossy(&out.stdout);
    let mut w = 0usize; let mut h = 0usize; let mut fps = 24.0f64;
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
    (w, h, fps)
}

/// 计算字符 alpha 包围盒（跨帧并集）。返回 (x0,y0,x1,y1)。
fn compute_union_bbox(src: &str, w: usize, h: usize, p: &P) -> Option<(usize, usize, usize, usize)> {
    let mut dec = Command::new("ffmpeg")
        .args(["-loglevel", "error", "-i", src, "-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
        .stdout(Stdio::piped())
        .spawn()
        .expect("启动 ffmpeg 解码失败");
    let frame = w * h * 3;
    let mut out = dec.stdout.take().unwrap();
    let mut buf = vec![0u8; frame];
    let mut bbox: Option<(usize, usize, usize, usize)> = None;
    let mut n = 0usize;
    loop {
        match out.read_exact(&mut buf) {
            Ok(_) => {}
            Err(_) => break,
        }
        // 只采样部分帧加速（每 3 帧取 1）
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

/// 单帧 RGB → RGBA（straight alpha），写入 out_rgba。
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

/// 计算 alpha 平面（用于包围盒，不做 RGBA 输出）。
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

/// 单像素 HSV 抠像 + 水印 + despill，返回 (r,g,b,alpha 0-255)。
#[inline]
fn key_px(r: f32, g: f32, b: f32, x: usize, y: usize, w: usize, h: usize, p: &P) -> (f32, f32, f32, f32) {
    let (hue, sat, val) = rgb_to_hsv(r, g, b);
    let in_hue = clamp((hue - p.lo) / p.feather, 0.0, 1.0) * clamp((p.hi - hue) / p.feather, 0.0, 1.0);
    let bg = sat >= p.sat_min && val >= p.val_min && hue >= p.lo && hue <= p.hi;
    let mut a = if bg { (1.0 - in_hue) * 255.0 } else { 255.0 };
    // 四角边距整块置透明（移除角落水印）
    if p.watermark && ((x < p.mw || x >= w - p.mw) && (y < p.mh || y >= h - p.mh)) {
        a = 0.0;
    }
    // despill：前景/边缘像素绿色过量 → 压回 max(R,B)
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
    let rn = r / 255.0; let gn = g / 255.0; let bn = b / 255.0;
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
fn clamp(x: f32, lo: f32, hi: f32) -> f32 { if x < lo { lo } else if x > hi { hi } else { x } }
