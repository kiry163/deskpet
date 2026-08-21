//! 临时验证工具：检查全部素材动画的首帧是否为 VP9 关键帧。
//! VP9 帧头 byte0 bit5 = frame_type（0=KEY_FRAME，1=INTER_FRAME）。
//! 若所有动画首帧（含 alpha）都是关键帧，则 seek(0) 无需重建解码器——
//! 直接解码关键帧即可重置参考帧状态（消除每次动画切换 ~6MB 的解码器泄漏）。
//! 用法：check_keyframe [assets_dir]（缺省同 decode_check 的自动解析）
#[path = "../webm.rs"]
mod webm;

use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    let assets_dir = args_assets_dir();
    println!("素材根: {}", assets_dir.display());
    let role_dir = resolve_character_dir(&assets_dir);
    let Some(role_dir) = role_dir else {
        println!("[FAIL] 找不到角色素材目录");
        std::process::exit(1);
    };
    println!("角色目录: {}", role_dir.display());

    let mut files: Vec<PathBuf> = Vec::new();
    collect_webm(&role_dir, &mut files);
    files.sort();
    println!("webm 数量: {}", files.len());

    let mut total = 0usize;
    let mut key = 0usize;
    let mut not_key: Vec<String> = Vec::new();
    let mut alpha_total = 0usize;
    let mut alpha_not_key: Vec<String> = Vec::new();

    for f in &files {
        let Some(data) = fs::read(f).ok() else { continue };
        let Some(wm) = webm::WebM::parse(&data) else { continue };
        let Some(f0) = wm.frames.first() else { continue };
        total += 1;
        if f0.video.is_empty() {
            not_key.push(format!("{}: 空视频帧", f.display()));
            continue;
        }
        let b0 = f0.video[0];
        let frame_type = (b0 >> 5) & 1; // 0 = KEY_FRAME
        if frame_type == 0 {
            key += 1;
        } else {
            not_key.push(format!("{}: frame_type={}", f.display(), frame_type));
        }
        if let Some(a) = &f0.alpha {
            alpha_total += 1;
            if !a.is_empty() {
                let ab0 = a[0];
                if ((ab0 >> 5) & 1) != 0 {
                    alpha_not_key.push(format!("{}: alpha frame_type={}", f.display(), (ab0 >> 5) & 1));
                }
            }
        }
    }
    println!("首帧关键帧: {}/{}", key, total);
    if !not_key.is_empty() {
        println!("非关键帧列表:");
        for s in &not_key {
            println!("  {}", s);
        }
    }
    println!("alpha 首帧总数: {}, 非关键帧: {}", alpha_total, alpha_not_key.len());
    for s in &alpha_not_key {
        println!("  {}", s);
    }
    println!("=== 结论: {}", if not_key.is_empty() && alpha_not_key.is_empty() { "全部首帧均为关键帧，可安全复用解码器" } else { "存在非关键帧首帧，需保留重建兜底" });
}

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
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")).join("assets")
}

fn resolve_character_dir(assets_dir: &Path) -> Option<PathBuf> {
    if let Ok(rd) = fs::read_dir(assets_dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() && contains_webm(&p) {
                return Some(p);
            }
        }
    }
    if contains_webm(assets_dir) {
        return Some(assets_dir.to_path_buf());
    }
    None
}

/// 递归含至少一个 *.webm。
fn contains_webm(dir: &Path) -> bool {
    let mut files: Vec<PathBuf> = Vec::new();
    collect_webm(dir, &mut files);
    !files.is_empty()
}

fn collect_webm(dir: &Path, out: &mut Vec<PathBuf>) {
    if let Ok(rd) = fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                collect_webm(&p, out);
            } else if p.extension().map_or(false, |x| x.eq_ignore_ascii_case("webm")) {
                out.push(p);
            }
        }
    }
}
