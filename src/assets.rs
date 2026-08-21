//! 素材加载：运行时从外部目录读取（与软件分离）。
//!
//! 素材包简化后（见 docs/需求规格.md §4）：素材集目录 = 若干 webm 平铺（任意目录层级），
//! **无 manifest.json / videos 结构要求**；动画名 = webm 文件名 stem（目录内唯一）；
//! 动作分类（触发类型）由管理端在 SQLite 中配置，加载时由调用方传入动作映射。
#![allow(dead_code)]

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use crate::webm::WebM;

/// 素材集（桌宠）。
pub struct RoleAssets {
    /// 动画名 → 解析后的 WebM（Rc 共享）。
    pub videos: HashMap<String, Rc<WebM>>,
    /// 动画名列表（videos 的键）。
    pub names: Vec<String>,
}

/// 素材根目录解析：配置 > 环境变量 DESKPET_ASSETS_DIR > 配置目录 assets/（若存在）>
/// exe 旁 assets/ > 当前目录 assets/ > 兜底配置目录 assets/（不存在则创建）。
/// 兜底保证 .app 从任意位置启动（Applications / Finder，cwd=/）时导入仍落到
/// 可写的配置目录，不会解析到只读路径（如 /assets）导致桌宠创建/导入失败。
pub fn resolve_assets_dir(configured: Option<&str>, config_dir: &Path) -> PathBuf {
    if let Some(d) = configured {
        if !d.trim().is_empty() {
            return PathBuf::from(d);
        }
    }
    if let Ok(d) = std::env::var("DESKPET_ASSETS_DIR") {
        if !d.trim().is_empty() {
            return PathBuf::from(d);
        }
    }
    let cfg_assets = config_dir.join("assets");
    if cfg_assets.is_dir() {
        return cfg_assets;
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
    let a = cwd.join("assets");
    if a.is_dir() {
        return a;
    }
    // 兜底：配置目录 assets/（首次导入前目录尚不存在也用它，并创建）
    let _ = std::fs::create_dir_all(&cfg_assets);
    cfg_assets
}

/// 从素材根目录加载素材集。失败返回 None（无目录/无素材/解析全部失败）。
pub fn load(assets_dir: &Path, character: Option<&str>) -> Option<RoleAssets> {
    log_debug!("素材根目录: {}", assets_dir.display());
    let dir = resolve_character_dir(assets_dir, character)?;
    log_info!("桌宠素材目录: {}", dir.display());
    load_from_dir(&dir)
}

/// 桌宠目录解析：character 指定 → assets_dir/character；否则第一个含 webm 的子目录；
/// 最后回退 assets_dir 本身（兼容扁平结构）。
fn resolve_character_dir(assets_dir: &Path, character: Option<&str>) -> Option<PathBuf> {
    if let Some(c) = character {
        let c = c.trim();
        if !c.is_empty() {
            let p = assets_dir.join(c);
            if is_character_dir(&p) {
                return Some(p);
            }
            log_warn!("指定桌宠素材目录不存在: {}", p.display());
            return None;
        }
    }
    if let Ok(rd) = fs::read_dir(assets_dir) {
        for e in rd.flatten() {
            if e.path().is_dir() && is_character_dir(&e.path()) {
                return Some(e.path());
            }
        }
    }
    if is_character_dir(assets_dir) {
        return Some(assets_dir.to_path_buf());
    }
    log_error!(
        "找不到桌宠素材目录（{}/ 下无含 webm 的子目录）",
        assets_dir.display()
    );
    None
}

/// 桌宠目录特征：递归含至少一个 *.webm。
fn is_character_dir(dir: &Path) -> bool {
    !scan_webm_files(dir).is_empty()
}

/// 列出素材目录内全部动画名（webm 文件名 stem，递归，同 stem 去重）。
pub fn scan_webm_names(role_dir: &Path) -> Vec<String> {
    scan_webm_files(role_dir).into_iter().map(|(n, _)| n).collect()
}

/// 递归收集目录内全部 *.webm → (文件名 stem, 路径)。同 stem 冲突保留第一个。
fn scan_webm_files(dir: &Path) -> Vec<(String, PathBuf)> {
    let mut out: Vec<(String, PathBuf)> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut stack: Vec<PathBuf> = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().map_or(false, |x| x.eq_ignore_ascii_case("webm")) {
                let stem = p.file_stem().unwrap_or_default().to_string_lossy().to_string();
                if !stem.is_empty() && seen.insert(stem.clone()) {
                    out.push((stem, p));
                }
            }
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// 从桌宠素材目录加载并解析全部 webm。
fn load_from_dir(dir: &Path) -> Option<RoleAssets> {
    let files = scan_webm_files(dir);
    if files.is_empty() {
        log_error!("素材目录无 webm: {}", dir.display());
        return None;
    }
    log_debug!("扫描到 {} 个 webm", files.len());

    let mut videos: HashMap<String, Rc<WebM>> = HashMap::new();
    let mut failed: Vec<String> = Vec::new();
    for (name, path) in &files {
        match fs::read(path) {
            Ok(data) => match WebM::parse(&data) {
                Some(wm) => {
                    videos.insert(name.clone(), Rc::new(wm));
                }
                None => failed.push(name.clone()),
            },
            Err(e) => log_warn!("读取素材失败 {}: {}", path.display(), e),
        }
    }
    if videos.is_empty() {
        log_error!("素材解析全部失败（共 {} 个 webm）", files.len());
        return None;
    }
    if !failed.is_empty() {
        log_warn!("{} 个素材解析失败: {:?}", failed.len(), failed);
    }
    log_info!("素材加载完成: {} 段动画", videos.len());

    let names: Vec<String> = videos.keys().cloned().collect();
    Some(RoleAssets { videos, names })
}
