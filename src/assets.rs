//! 素材加载：运行时从外部目录读取（与软件分离）。
//!
//! 目录约定（见 docs/需求规格.md §1.3）：
//! ```text
//! <assets_dir>/<角色>/
//! ├── manifest.json          # 角色元数据 + 动作分类（可缺省）
//! └── videos/                # 动作 webm；可用动作语义子目录（idle/turn/move/click/drag/random）
//! ```
//! 角色解析：配置 `character` 指定 → `<assets_dir>/<character>`；
//! 否则取 `<assets_dir>` 下第一个含 `videos/` 或 `manifest.json` 的子目录；
//! 最后回退 `<assets_dir>` 本身（兼容扁平结构）。
#![allow(dead_code)]

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use crate::webm::WebM;

/// 素材集（角色）。
pub struct RoleAssets {
    /// 动画名 → 解析后的 WebM（Rc 共享）。
    pub videos: HashMap<String, Rc<WebM>>,
    /// videos/ 子目录 → 动画名列表（无子目录时为空 → flat 分类）。
    pub folder_files: HashMap<String, Vec<String>>,
    /// manifest.json 内容（无则 None）。
    pub manifest: Option<serde_json::Value>,
    /// 动画名列表（videos 的键）。
    pub names: Vec<String>,
}

/// 素材根目录解析：配置 > 环境变量 DESKPET_ASSETS_DIR > 配置目录 assets/（若存在）>
/// exe 旁 assets/ > 当前目录 assets/。默认素材根 = 配置目录 assets/（M1 决策，
/// 发布物仅二进制后 exe 旁可能只读；首次导入后该目录即存在）。
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
    a
}

/// 从素材根目录加载角色。失败返回 None（无目录/无素材/解析全部失败）。
pub fn load(assets_dir: &Path, character: Option<&str>) -> Option<RoleAssets> {
    log_debug!("素材根目录: {}", assets_dir.display());
    let dir = resolve_character_dir(assets_dir, character)?;
    log_info!("角色素材目录: {}", dir.display());
    load_from_dir(&dir)
}

/// 角色目录解析：character 指定 → assets_dir/character；否则第一个含 videos/ 或
/// manifest.json 的子目录；最后回退 assets_dir 本身（兼容扁平结构）。
fn resolve_character_dir(assets_dir: &Path, character: Option<&str>) -> Option<PathBuf> {
    if let Some(c) = character {
        let c = c.trim();
        if !c.is_empty() {
            let p = assets_dir.join(c);
            if is_character_dir(&p) {
                return Some(p);
            }
            log_warn!("指定角色目录不存在: {}", p.display());
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
        "找不到角色素材目录（{}/ 下无含 videos/ 或 manifest.json 的子目录）",
        assets_dir.display()
    );
    None
}

/// 角色目录特征：含 videos/ 子目录或 manifest.json。
fn is_character_dir(dir: &Path) -> bool {
    dir.join("videos").is_dir() || dir.join("manifest.json").is_file()
}

/// 扫描 videos/：收集全部 webm（顶层 flat + 动作子目录），返回 (文件列表, 子目录分类)。
fn scan_videos(dir: &Path) -> (Vec<(String, PathBuf)>, HashMap<String, Vec<String>>) {
    let videos = dir.join("videos");
    let mut files: Vec<(String, PathBuf)> = Vec::new();
    let mut folder_files: HashMap<String, Vec<String>> = HashMap::new();
    if !videos.is_dir() {
        log_warn!("缺少 videos/ 子目录: {}", videos.display());
        return (files, folder_files);
    }
    // 动作子目录（idle/turn/move/click/drag/random 等）
    if let Ok(rd) = fs::read_dir(&videos) {
        for e in rd.flatten() {
            let p = e.path();
            if !p.is_dir() {
                continue;
            }
            let folder = p.file_name().unwrap().to_string_lossy().to_string();
            let mut sub: Vec<PathBuf> = fs::read_dir(&p)
                .map(|rd2| rd2.flatten().map(|e| e.path()).collect())
                .unwrap_or_default();
            sub.sort();
            let mut names: Vec<String> = Vec::new();
            for f in sub {
                if f.extension().map_or(false, |x| x == "webm") {
                    let name = f.file_stem().unwrap().to_string_lossy().to_string();
                    names.push(name.clone());
                    files.push((name, f));
                }
            }
            if !names.is_empty() {
                folder_files.insert(folder, names);
            }
        }
    }
    // 顶层 webm（flat 结构）
    let mut top: Vec<PathBuf> = fs::read_dir(&videos)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| p.is_file() && p.extension().map_or(false, |x| x == "webm"))
                .collect()
        })
        .unwrap_or_default();
    top.sort();
    for f in top {
        let name = f.file_stem().unwrap().to_string_lossy().to_string();
        files.push((name, f));
    }
    (files, folder_files)
}

/// 从角色目录加载并解析全部 webm。
fn load_from_dir(dir: &Path) -> Option<RoleAssets> {
    let (files, folder_files) = scan_videos(dir);
    if files.is_empty() {
        log_error!("素材目录无 webm: {}", dir.join("videos").display());
        return None;
    }
    log_debug!("扫描到 {} 个 webm", files.len());

    // manifest.json（可选）
    let manifest = fs::read_to_string(dir.join("manifest.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok());
    if manifest.is_some() {
        log_debug!("读取 manifest.json");
    }

    // 解析 webm
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
    Some(RoleAssets {
        videos,
        folder_files,
        manifest,
        names,
    })
}
