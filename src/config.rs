//! 配置分层（见 docs/需求规格.md §2）：
//!
//! - **系统级**：单一 YAML `deskpet.yaml`（引导级，打开数据库前必须已知）——
//!   `database_path / assets_dir / console_port / log_level`；首次运行自动生成默认文件，
//!   用户可提前放置或手改；
//! - **程序级**：SQLite `app_settings`（外观 / 位置 / 当前桌宠等，控制台可编辑、热生效）；
//! - **桌宠级**：SQLite `pets` + `pet_actions`（见 `db.rs`）。
//!
//! 不做旧版 config.json 迁移（未正式发布，无历史用户）。
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::db::Db;

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

/// 系统级配置（唯一 YAML，引导级）。
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct SystemConfig {
    /// 数据库文件路径；None = 默认 `<配置目录>/deskpet.db`；不存在则自动建库。
    /// 支持绝对路径或相对配置目录的相对路径。
    pub database_path: Option<String>,
    /// 素材根目录；None = 自动解析（配置目录 assets/ 优先）。
    pub assets_dir: Option<String>,
    /// 控制台 HTTP 端口；None = 默认 18686（冲突回退随机）。
    pub console_port: Option<u16>,
    /// 日志级别（off|error|warn|info|debug）；None = 默认 info（环境变量 DESKPET_LOG 优先）。
    pub log_level: Option<String>,
}

impl Default for SystemConfig {
    fn default() -> Self {
        SystemConfig {
            database_path: None,
            assets_dir: None,
            console_port: None,
            log_level: None,
        }
    }
}

/// 程序级配置（SQLite app_settings；内存态，save() 落库）。
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
    /// 当前桌宠（素材集 id）；None = 未导入。
    pub character: Option<String>,
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
            character: None,
        }
    }
}

/// 应用配置聚合：目录 + YAML（系统级）+ SQLite（程序级/桌宠级）。
pub struct Config {
    /// 配置目录（`<平台配置根>/deskpet/`）。
    pub dir: PathBuf,
    /// YAML 文件路径。
    #[allow(dead_code)]
    pub yaml_path: PathBuf,
    /// 系统级（YAML）。
    pub sys: SystemConfig,
    /// 数据库（打开失败时内存库兜底，配置不持久）。
    pub db: Db,
    /// 程序级（app_settings，内存态）。
    pub pet: PetConfig,
}

impl Config {
    pub fn load() -> Config {
        let base = config_base_dir();
        let dir = base.join("deskpet");
        if let Err(e) = fs::create_dir_all(&dir) {
            log_error!("创建配置目录失败 {}: {}", dir.display(), e);
        }
        let yaml_path = dir.join("deskpet.yaml");

        // 1. YAML：不存在 → 生成默认；存在 → 解析（失败用默认值并告警）
        let sys = if yaml_path.exists() {
            match fs::read_to_string(&yaml_path)
                .map_err(|e| format!("{}", e))
                .and_then(|t| serde_yaml::from_str::<SystemConfig>(&t).map_err(|e| format!("{}", e)))
            {
                Ok(s) => {
                    log_debug!("读取配置: {}", yaml_path.display());
                    s
                }
                Err(e) => {
                    log_warn!("配置文件解析失败，使用默认值 {}: {}", yaml_path.display(), e);
                    SystemConfig::default()
                }
            }
        } else {
            let default = SystemConfig {
                database_path: Some(default_db_path(&dir)),
                ..Default::default()
            };
            let text = serde_yaml::to_string(&default).unwrap_or_default();
            let header = "# deskpet 系统级配置（引导级）。\n\
                          # 本文件只配置应用启动前必须知道的项；其余配置存 SQLite（deskpet.db）。\n\
                          # 首次运行前可手动放置本文件以指定数据库路径（不存在则自动建库）。\n\n";
            let _ = fs::write(&yaml_path, format!("{}{}", header, text));
            log_info!("已生成默认配置: {}", yaml_path.display());
            default
        };

        // 2. 日志级别：环境变量优先，其次 YAML
        if std::env::var("DESKPET_LOG").is_err() {
            if let Some(lv) = sys.log_level.as_deref() {
                crate::log::set_level_str(lv);
            }
        }

        // 3. SQLite：打开（无库自动建表）；失败退化为内存库，保证桌宠可用
        let db_path = resolve_db_path(sys.database_path.as_deref(), &dir);
        let db = match Db::open(&db_path) {
            Ok(db) => db,
            Err(e) => {
                log_error!("{}（退化为内存数据库，配置不持久）", e);
                Db::open_in_memory()
            }
        };

        // 4. 程序级配置：从 app_settings 恢复（缺省用默认值）
        let pet = load_pet_config(&db);

        Config { dir, yaml_path, sys, db, pet }
    }

    /// 保存程序级配置到 SQLite。
    pub fn save(&self) {
        let db = &self.db;
        let _ = db.set_setting("rx", &json_or_null(self.pet.rx));
        let _ = db.set_setting("ry", &json_or_null(self.pet.ry));
        let _ = db.set_setting("facing_right", &format!("{}", self.pet.facing_right));
        let _ = db.set_setting("scale", &format!("{}", self.pet.scale));
        let _ = db.set_setting("always_on_top", &format!("{}", self.pet.always_on_top));
        let _ = db.set_setting("no_move", &format!("{}", self.pet.no_move));
        let _ = db.set_setting(
            "character",
            &self
                .pet
                .character
                .as_deref()
                .map(|s| format!("\"{}\"", s))
                .unwrap_or_else(|| "null".to_string()),
        );
        log_debug!("程序级配置已保存到数据库");
    }
}

fn json_or_null(v: Option<f64>) -> String {
    match v {
        Some(x) => format!("{}", x),
        None => "null".to_string(),
    }
}

/// 从 app_settings 恢复程序级配置（缺省 = 默认值）。
fn load_pet_config(db: &Db) -> PetConfig {
    let s = db.all_settings().unwrap_or_default();
    let mut pc = PetConfig::default();
    if let Some(v) = s.get("rx").and_then(|x| x.parse::<f64>().ok()) {
        pc.rx = Some(v);
    }
    if let Some(v) = s.get("ry").and_then(|x| x.parse::<f64>().ok()) {
        pc.ry = Some(v);
    }
    if let Some(v) = s.get("facing_right").and_then(|x| x.parse::<bool>().ok()) {
        pc.facing_right = v;
    }
    if let Some(v) = s.get("scale").and_then(|x| x.parse::<f64>().ok()) {
        pc.scale = v;
    }
    if let Some(v) = s.get("always_on_top").and_then(|x| x.parse::<bool>().ok()) {
        pc.always_on_top = v;
    }
    if let Some(v) = s.get("no_move").and_then(|x| x.parse::<bool>().ok()) {
        pc.no_move = v;
    }
    if let Some(v) = s.get("character") {
        let c = v.trim().trim_matches('"');
        if !c.is_empty() && c != "null" {
            pc.character = Some(c.to_string());
        }
    }
    pc
}

/// 数据库路径解析：YAML 指定（绝对直接用 / 相对基于配置目录）→ 默认 `<配置目录>/deskpet.db`。
fn resolve_db_path(configured: Option<&str>, dir: &Path) -> PathBuf {
    if let Some(p) = configured {
        let p = p.trim();
        if !p.is_empty() {
            let pb = PathBuf::from(p);
            return if pb.is_absolute() { pb } else { dir.join(pb) };
        }
    }
    dir.join("deskpet.db")
}

fn default_db_path(dir: &Path) -> String {
    dir.join("deskpet.db").to_string_lossy().to_string()
}
