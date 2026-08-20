//! 配置持久化：Windows %APPDATA%\deskpet\config.json；macOS ~/Library/Application Support/deskpet/config.json。
//! 首次运行（本应用无配置时）会尝试从旧版 Tauri 应用配置迁移 scale/置顶/不移动/朝向/自启。
use std::env;
use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// 配置根目录（平台相关）。
#[cfg(windows)]
fn config_base_dir() -> PathBuf {
    env::var("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| env::var("USERPROFILE").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from(".")))
}

#[cfg(target_os = "macos")]
fn config_base_dir() -> PathBuf {
    env::var("HOME")
        .map(|h| PathBuf::from(h).join("Library").join("Application Support"))
        .unwrap_or_else(|_| PathBuf::from("."))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct PetConfig {
    /// 工作区内水平归一化位置（0..1，None = 默认右下角）。
    pub rx: Option<f64>,
    /// 工作区内垂直归一化位置（0..1）。
    pub ry: Option<f64>,
    pub facing_right: bool,
    pub scale: f64,
    pub always_on_top: bool,
    pub no_move: bool,
}

impl Default for PetConfig {
    fn default() -> Self {
        PetConfig {
            rx: None,
            ry: None,
            facing_right: false,
            scale: 0.72,
            always_on_top: true,
            no_move: false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Config {
    pub dir: PathBuf,
    pub pet: PetConfig,
}

impl Config {
    pub fn load() -> Config {
        let base = config_base_dir();
        let dir = base.join("deskpet");
        let path = dir.join("config.json");
        let mut cfg = Config {
            dir,
            pet: PetConfig::default(),
        };
        if let Ok(text) = fs::read_to_string(&path) {
            if let Ok(pet) = serde_json::from_str::<PetConfig>(&text) {
                cfg.pet = pet;
                return cfg;
            }
        }
        cfg.migrate_from_tauri();
        cfg
    }

    /// 旧版 Tauri 应用配置迁移（仅当本应用没有配置文件时执行）。
    fn migrate_from_tauri(&mut self) {
        let legacy = config_base_dir().join("com.kiry.deskpet").join("config.json");
        let Ok(text) = fs::read_to_string(&legacy) else { return };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else { return };
        if let Some(s) = v.get("scale").and_then(|x| x.as_f64()) {
            self.pet.scale = s;
        }
        if let Some(b) = v.get("no_move").and_then(|x| x.as_bool()) {
            self.pet.no_move = b;
        }
        if let Some(b) = v.get("always_on_top").and_then(|x| x.as_bool()) {
            self.pet.always_on_top = b;
        }
        if let Some(b) = v.get("facing_right").and_then(|x| x.as_bool()) {
            self.pet.facing_right = b;
        }
        if v.get("autostart").and_then(|x| x.as_bool()) == Some(true) {
            crate::autostart::set_enabled(true);
        }
        // 旧版 x/y 是绝对像素坐标，不迁移（原生版用归一化 rx/ry，位置重新确定）
    }

    pub fn save(&self) {
        let _ = fs::create_dir_all(&self.dir);
        let json = serde_json::to_string_pretty(&self.pet).unwrap_or_default();
        let _ = fs::write(self.dir.join("config.json"), json);
    }
}
