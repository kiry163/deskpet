//! 嵌入素材（build.rs 生成 assets.pak + assets_gen.rs）+ 内置素材集。
#![allow(dead_code)]
use std::collections::HashMap;
use std::rc::Rc;

use crate::webm::WebM;

include!(concat!(env!("OUT_DIR"), "/assets_gen.rs"));

/// 按动画名取 webm 字节。
pub fn asset(name: &str) -> Option<&'static [u8]> {
    for (n, start, len) in ANIMS {
        if *n == name {
            return Some(&ASSET_PAK[*start..*start + len]);
        }
    }
    None
}

/// 内置素材集（单角色、51 个动画、flat 结构）。
pub struct RoleAssets {
    /// 动画名 → 解析后的 WebM（Rc 共享）。
    pub videos: HashMap<String, Rc<WebM>>,
    /// videos/ 子目录 → 动画名列表（内置素材无子目录，恒为空）。
    pub folder_files: HashMap<String, Vec<String>>,
    /// manifest.json 内容（内置素材无）。
    pub manifest: Option<serde_json::Value>,
    /// 动画名列表（videos 的键）。
    pub names: Vec<String>,
}

/// 从内嵌 ASSET_PAK 加载全部 webm。
pub fn load_builtin() -> Option<RoleAssets> {
    log_debug!("加载内置素材（{} 段动画）", ANIMS.len());
    let mut videos: HashMap<String, Rc<WebM>> = HashMap::new();
    let mut failed: Vec<&str> = Vec::new();
    for (name, start, len) in ANIMS {
        let data = &ASSET_PAK[*start..*start + *len];
        if let Some(wm) = WebM::parse(data) {
            videos.insert(name.to_string(), Rc::new(wm));
        } else {
            failed.push(name);
        }
    }
    if videos.is_empty() {
        log_error!("素材解析全部失败（共 {} 段），无法启动", ANIMS.len());
        return None;
    }
    if !failed.is_empty() {
        log_warn!("{} 段素材解析失败: {:?}", failed.len(), failed);
    }
    log_info!("素材加载完成: {} 段动画", videos.len());
    let names: Vec<String> = videos.keys().cloned().collect();
    Some(RoleAssets {
        videos,
        folder_files: HashMap::new(),
        manifest: None,
        names,
    })
}
