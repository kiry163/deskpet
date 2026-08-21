//! 解码管线验证：解析 webm → libvpx 解码 → 合成 BGRA，检查 alpha 正确性。
//! 用法：`decode_check [assets_dir]`（缺省自动解析素材根，同主程序：配置/环境变量/exe 旁/cwd）
#[path = "../webm.rs"]
mod webm;
#[path = "../vpx.rs"]
mod vpx;
#[path = "../clip.rs"]
mod clip;

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

fn main() {
    let assets_dir = args_assets_dir();
    println!("素材根: {}", assets_dir.display());
    let Some(role_dir) = resolve_character_dir(&assets_dir) else {
        println!("[FAIL] 找不到角色素材目录");
        std::process::exit(1);
    };
    println!("角色目录: {}", role_dir.display());
    let (files, _folders) = scan_videos(&role_dir);
    if files.is_empty() {
        println!("[FAIL] 素材目录无 webm");
        std::process::exit(1);
    }

    // 1. 全部动画都能解析出帧
    let mut total_frames = 0usize;
    let mut ok = 0;
    let mut fail = 0;
    for (name, path) in &files {
        match fs::read(path).ok().and_then(|d| webm::WebM::parse(&d)) {
            Some(wm) => {
                ok += 1;
                total_frames += wm.frames.len();
                if wm.frames.is_empty() {
                    println!("[WARN] {} 无帧", name);
                }
            }
            None => {
                fail += 1;
                if fail <= 3 {
                    println!("[FAIL] {} 解析失败", name);
                }
            }
        }
    }
    println!("=== 解析成功 {} / {}，总帧数 {} ===", ok, files.len(), total_frames);

    // 2. 待机动画解码第一帧，验证 alpha
    let target = "待机呼吸休闲";
    let Some((_, path)) = files.iter().find(|(n, _)| n == target) else {
        println!("[FAIL] 找不到待机动画: {}", target);
        std::process::exit(1);
    };
    let data = fs::read(path).unwrap();
    let wm = webm::WebM::parse(&data).unwrap();
    println!("{}: {} 帧, fps={:.1}, 尺寸 {}x{}", target, wm.frames.len(), wm.fps, wm.width, wm.height);

    let mut dec = clip::ClipDecoder::new(Rc::new(wm)).unwrap();
    // 共享解码器方案下解码器由 Pet 级持有；此处自建一组用于独立验证
    let mut color_dec = vpx::Decoder::new(4).unwrap();
    let mut alpha_dec = vpx::Decoder::new(2).unwrap();
    let mut comp_buf: Vec<u8> = Vec::new();

    // 直接解码 image 结构检查
    {
        let wm2 = webm::WebM::parse(&data).unwrap();
        let f0 = &wm2.frames[0];
        let mut vdec = vpx::Decoder::new(4).unwrap();
        let img = vdec.decode(&f0.video).unwrap();
        println!("image: w={} h={} d_w={} d_h={} fmt=0x{:x} bitdepth={} xcs={} ycs={} stride={:?}",
            img.w, img.h, img.d_w, img.d_h, img.fmt, img.bit_depth, img.x_chroma_shift, img.y_chroma_shift, img.stride);
        if let Some(a) = &f0.alpha {
            let mut adec = vpx::Decoder::new(2).unwrap();
            let aimg = adec.decode(a).unwrap();
            println!("alpha image: w={} h={} d_w={} d_h={} fmt=0x{:x} stride={:?}",
                aimg.w, aimg.h, aimg.d_w, aimg.d_h, aimg.fmt, aimg.stride);
        }
    }

    let t0 = std::time::Instant::now();
    let frame = dec.next_frame(&mut color_dec, Some(&mut alpha_dec), &mut comp_buf).expect("解码第一帧失败");
    let dec_ms = t0.elapsed().as_millis();
    println!("第一帧解码耗时: {}ms, 缓冲 {}B", dec_ms, frame.len());

    let mut a0 = 0u32;
    let mut a_lt128 = 0u32;
    let mut a255 = 0u32;
    let mut total = 0u32;
    for i in (3..frame.len()).step_by(4) {
        let a = frame[i];
        total += 1;
        if a == 0 { a0 += 1; }
        if a < 128 { a_lt128 += 1; }
        if a >= 250 { a255 += 1; }
    }
    println!("alpha 分布: total={} a=0:{:.1}% a<128:{:.1}% a>=250:{:.1}%",
        total,
        a0 as f64 * 100.0 / total as f64,
        a_lt128 as f64 * 100.0 / total as f64,
        a255 as f64 * 100.0 / total as f64,
    );

    // 非全透明（有实际画面）
    assert!(a255 > 0, "没有完全不透明像素，解码可能失败");
    assert!(a0 < total, "全透明帧");

    // 3. 连续解码 10 帧测性能
    let t0 = std::time::Instant::now();
    let mut n = 0;
    while let Some(_) = dec.next_frame(&mut color_dec, Some(&mut alpha_dec), &mut comp_buf) {
        n += 1;
        if n >= 20 {
            break;
        }
    }
    let dur = t0.elapsed().as_millis();
    println!("连续解码 {} 帧耗时 {}ms, 平均 {:.2}ms/帧", n, dur, dur as f64 / n as f64);
    assert!((dur as f64 / n as f64) < 40.0, "解码性能不足，达不到 24fps");
    println!("\n=== 解码管线验证通过 ===");
}

/// 素材根：命令行参数 > DESKPET_ASSETS_DIR > exe 旁 assets/ > 当前目录 assets/。
fn args_assets_dir() -> PathBuf {
    if let Some(a) = std::env::args().nth(1) {
        return PathBuf::from(a);
    }
    if let Ok(d) = std::env::var("DESKPET_ASSETS_DIR") {
        if !d.trim().is_empty() {
            return PathBuf::from(d);
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(p) = exe.parent() {
            let a = p.join("assets");
            if a.is_dir() {
                return a;
            }
        }
    }
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    cwd.join("assets")
}

/// 角色目录：第一个含 videos/ 或 manifest.json 的子目录；否则自身。
fn resolve_character_dir(assets_dir: &Path) -> Option<PathBuf> {
    if let Ok(rd) = fs::read_dir(assets_dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() && (p.join("videos").is_dir() || p.join("manifest.json").is_file()) {
                return Some(p);
            }
        }
    }
    if assets_dir.join("videos").is_dir() || assets_dir.join("manifest.json").is_file() {
        return Some(assets_dir.to_path_buf());
    }
    None
}

/// 扫描 videos/：顶层 flat + 动作子目录，返回 (name, path) 列表与子目录分类。
fn scan_videos(dir: &Path) -> (Vec<(String, PathBuf)>, HashMap<String, Vec<String>>) {
    let videos = dir.join("videos");
    let mut files: Vec<(String, PathBuf)> = Vec::new();
    let mut folders: HashMap<String, Vec<String>> = HashMap::new();
    if !videos.is_dir() {
        return (files, folders);
    }
    if let Ok(rd) = fs::read_dir(&videos) {
        for e in rd.flatten() {
            let p = e.path();
            if !p.is_dir() {
                continue;
            }
            let folder = p.file_name().unwrap().to_string_lossy().to_string();
            let mut names = Vec::new();
            if let Ok(rd2) = fs::read_dir(&p) {
                let mut sub: Vec<PathBuf> = rd2.flatten().map(|e| e.path()).filter(|p| p.extension().map_or(false, |x| x == "webm")).collect();
                sub.sort();
                for f in sub {
                    let name = f.file_stem().unwrap().to_string_lossy().to_string();
                    names.push(name.clone());
                    files.push((name, f));
                }
            }
            if !names.is_empty() {
                folders.insert(folder, names);
            }
        }
    }
    if let Ok(rd) = fs::read_dir(&videos) {
        let mut top: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.is_file() && p.extension().map_or(false, |x| x == "webm")).collect();
        top.sort();
        for f in top {
            let name = f.file_stem().unwrap().to_string_lossy().to_string();
            files.push((name, f));
        }
    }
    (files, folders)
}
