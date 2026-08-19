//! webm2frames —— VP9+alpha webm → RGBA 帧序列转换工具。
//!
//! 用法：
//!   webm2frames <input.webm> <output_dir> [--fps 12] [--format webp|png] [--quality 80] [--prefix f]
//!
//! 说明：
//! - 解码模块（webm.rs / vpx.rs / clip.rs）移植自 ianlike-ui/dsh-pet-standalone（MIT），
//!   其素材与逻辑源自 MIT 协议的 PC2005-cloud/dsh-pet。
//! - 素材是「主色 yuv420p + BlockAdditional alpha」结构的 VP9 webm，ffmpeg 命令行会丢弃
//!   alpha，必须用 libvpx 双路解码（本工具）。
//! - clip.rs 输出 premultiplied BGRA，本工具转回 straight-alpha RGBA 后写 PNG/WebP。

mod webm;
mod vpx;
mod clip;

use std::fs;
use std::path::Path;
use std::rc::Rc;
use std::time::Instant;

use image::{ExtendedColorType, ImageEncoder, RgbaImage};
use image::codecs::png::PngEncoder;
use image::codecs::webp::WebPEncoder;

struct Args {
    input: String,
    out_dir: String,
    fps: f64,          // 目标输出帧率；<=0 表示全部帧
    format: String,    // "png" | "webp"
    prefix: String,
}

fn parse_args() -> Result<Args, String> {
    let mut args = std::env::args().skip(1);
    let input = args.next().ok_or("用法: webm2frames <input.webm> <output_dir> [--fps N] [--format webp|png] [--prefix P]")?;
    let out_dir = args.next().ok_or("缺少输出目录参数")?;
    let mut fps = 0.0f64;
    let mut format = "webp".to_string();
    let mut prefix = "f".to_string();
    while let Some(a) = args.next() {
        match a.as_str() {
            "--fps" => fps = args.next().ok_or("--fps 需要值")?.parse().map_err(|_| "--fps 不是数字")?,
            "--format" => format = args.next().ok_or("--format 需要值")?.to_lowercase(),
            "--prefix" => prefix = args.next().ok_or("--prefix 需要值")?,
            _ => return Err(format!("未知参数: {}", a)),
        }
    }
    if format != "png" && format != "webp" {
        return Err("format 只支持 png 或 webp".into());
    }
    Ok(Args { input, out_dir, fps, format, prefix })
}

/// premultiplied BGRA → straight-alpha RGBA。
fn bgra_premul_to_rgba_straight(buf: &[u8], w: usize, h: usize) -> Vec<u8> {
    let mut out = vec![0u8; w * h * 4];
    for (i, px) in buf.chunks_exact(4).enumerate() {
        let (b, g, r, a) = (px[0] as u32, px[1] as u32, px[2] as u32, px[3] as u32);
        if a == 0 {
            out[i * 4..i * 4 + 4].copy_from_slice(&[0, 0, 0, 0]);
        } else {
            // c = (c_premul * 255 + a/2) / a，四舍五入回 straight
            out[i * 4] = ((r * 255 + a / 2) / a) as u8;
            out[i * 4 + 1] = ((g * 255 + a / 2) / a) as u8;
            out[i * 4 + 2] = ((b * 255 + a / 2) / a) as u8;
            out[i * 4 + 3] = a as u8;
        }
    }
    out
}

fn write_frame(path: &Path, rgba: &[u8], w: u32, h: u32, format: &str) -> Result<(), String> {
    let file = fs::File::create(path).map_err(|e| format!("创建 {} 失败: {}", path.display(), e))?;
    // 注：image crate 的 WebP 编码器仅支持无损模式（对动画帧扁平色块压缩率尚可）。
    let result = if format == "png" {
        let enc = PngEncoder::new(file);
        enc.write_image(rgba, w, h, ExtendedColorType::Rgba8)
    } else {
        let enc = WebPEncoder::new_lossless(file);
        enc.write_image(rgba, w, h, ExtendedColorType::Rgba8)
    };
    result.map_err(|e| format!("编码 {} 失败: {}", path.display(), e))
}

fn main() {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{}", e);
            std::process::exit(2);
        }
    };

    // 1. 读取并解析 webm
    let data = fs::read(&args.input).unwrap_or_else(|e| {
        eprintln!("读取 {} 失败: {}", args.input, e);
        std::process::exit(1);
    });
    let wm = match webm::WebM::parse(&data) {
        Some(w) => w,
        None => {
            eprintln!("解析 {} 失败（不是有效的 VP9+alpha webm？）", args.input);
            std::process::exit(1);
        }
    };
    let (w, h) = (wm.width as usize, wm.height as usize);
    let src_fps = wm.fps;
    println!(
        "解析成功: {}x{}, 源帧率 {:.1}fps, {} 帧, 时长 {:.2}s, alpha 帧 {}",
        w, h, src_fps, wm.frames.len(),
        wm.duration_ms() as f64 / 1000.0,
        wm.frames.iter().filter(|f| f.alpha.is_some()).count(),
    );

    // 2. 计算抽帧步长
    let step = if args.fps > 0.0 && args.fps < src_fps {
        (src_fps / args.fps).round().max(1.0) as usize
    } else {
        1
    };
    if step > 1 {
        println!("抽帧: {}fps → {}fps（每 {} 帧取 1）", src_fps, args.fps, step);
    }

    // 3. 创建输出目录
    fs::create_dir_all(&args.out_dir).unwrap_or_else(|e| {
        eprintln!("创建输出目录 {} 失败: {}", args.out_dir, e);
        std::process::exit(1);
    });

    // 4. 逐帧解码 + 转换 + 写出
    let wm_rc = Rc::new(wm);
    let mut dec = match clip::ClipDecoder::new(wm_rc.clone()) {
        Some(d) => d,
        None => {
            eprintln!("初始化解码器失败（libvpx 是否已安装？）");
            std::process::exit(1);
        }
    };

    let mut written = 0usize;
    let mut total_bytes = 0u64;
    let t0 = Instant::now();
    let mut idx = 0usize;
    while let Some(bgra) = dec.next_frame() {
        if idx % step == 0 {
            let rgba = bgra_premul_to_rgba_straight(&bgra, w, h);
            let name = format!("{}{:05}.{}", args.prefix, written, args.format);
            let path = Path::new(&args.out_dir).join(name);
            if let Err(e) = write_frame(&path, &rgba, w as u32, h as u32, &args.format) {
                eprintln!("{}", e);
                std::process::exit(1);
            }
            total_bytes += fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            written += 1;
        }
        idx += 1;
    }
    let dur = t0.elapsed();

    println!(
        "完成: 写出 {} 帧到 {}，共 {:.1} MB（{:.1} KB/帧），耗时 {:.2}s（{:.1}ms/帧）",
        written,
        args.out_dir,
        total_bytes as f64 / 1048576.0,
        total_bytes as f64 / written.max(1) as f64 / 1024.0,
        dur.as_secs_f64(),
        dur.as_secs_f64() * 1000.0 / written.max(1) as f64,
    );
}
