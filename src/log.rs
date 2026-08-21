//! 轻量文件日志：写入 `<配置目录>/logs/deskpet.log`。
//!
//! - 滚动：单文件超过 1MB → 改名 `deskpet.log.old`（覆盖旧备份），新文件重写；
//!   磁盘占用 ≤2MB，无需后台任务（每次写入前检查）。
//! - 级别过滤：环境变量 `DESKPET_LOG`（off|error|warn|info|debug，默认 info）。
//! - 每次写入后 flush（日志量小，每秒几行，无性能压力）。
#![allow(dead_code)]

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU8, Ordering};

/// 单文件大小上限（超过即滚动）。
const MAX_BYTES: u64 = 1024 * 1024;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Error = 0,
    Warn = 1,
    Info = 2,
    Debug = 3,
}

impl Level {
    fn as_str(self) -> &'static str {
        match self {
            Level::Error => "ERROR",
            Level::Warn => "WARN",
            Level::Info => "INFO",
            Level::Debug => "DEBUG",
        }
    }
}

static LEVEL: AtomicU8 = AtomicU8::new(Level::Info as u8);

/// 初始化日志：读取级别环境变量并写入启动标记。
pub fn init() {
    if let Ok(v) = std::env::var("DESKPET_LOG") {
        let lv = match v.trim().to_ascii_lowercase().as_str() {
            "off" => 255,
            "error" => Level::Error as u8,
            "warn" => Level::Warn as u8,
            "info" => Level::Info as u8,
            "debug" => Level::Debug as u8,
            _ => Level::Info as u8,
        };
        LEVEL.store(lv, Ordering::Relaxed);
    }
    write(
        Level::Info,
        &format!("==== deskpet {} 启动 ====", env!("CARGO_PKG_VERSION")),
    );
}

/// 当前级别是否启用。
pub fn enabled(lv: Level) -> bool {
    (lv as u8) <= LEVEL.load(Ordering::Relaxed)
}

/// 解析级别字符串并设置（off|error|warn|info|debug，未知值忽略）。
pub fn set_level_str(s: &str) {
    let lv = match s.trim().to_ascii_lowercase().as_str() {
        "off" => 255,
        "error" => Level::Error as u8,
        "warn" => Level::Warn as u8,
        "info" => Level::Info as u8,
        "debug" => Level::Debug as u8,
        _ => return,
    };
    LEVEL.store(lv, Ordering::Relaxed);
    let _ = write(Level::Info, &format!("日志级别已配置为 {}", s.trim()));
}

/// 写一条日志（追加；超限先滚动）。
pub fn write(lv: Level, msg: &str) {
    if !enabled(lv) {
        return;
    }
    let line = format!("[{}] [{:<5}] {}\n", timestamp(), lv.as_str(), msg);
    let path = log_path();
    // 滚动：超过上限 → 当前文件改名为 .old（覆盖旧备份）
    if fs::metadata(&path).map(|m| m.len() > MAX_BYTES).unwrap_or(false) {
        let _ = fs::rename(&path, path.with_extension("log.old"));
    }
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&path) {
        let _ = f.write_all(line.as_bytes());
        let _ = f.flush();
    }
}

/// 日志文件路径：`<配置目录>/logs/deskpet.log`。
fn log_path() -> PathBuf {
    base_dir().join("deskpet").join("logs").join("deskpet.log")
}

/// 平台配置根目录（与 config.rs 的 base dir 一致）。
fn base_dir() -> PathBuf {
    #[cfg(windows)]
    {
        std::env::var("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                std::env::var("USERPROFILE").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("."))
            })
    }
    #[cfg(target_os = "macos")]
    {
        std::env::var("HOME")
            .map(|h| PathBuf::from(h).join("Library").join("Application Support"))
            .unwrap_or_else(|_| PathBuf::from("."))
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        PathBuf::from(".")
    }
}

fn timestamp() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let d = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = d.as_secs();
    let (h, m, s) = (secs / 3600 % 24, secs / 60 % 60, secs % 60);
    format!("{:02}:{:02}:{:02}.{:03}", h, m, s, d.subsec_millis())
}

/// 日志宏（ERROR / WARN / INFO / DEBUG）。
#[macro_export]
macro_rules! log_error { ($($t:tt)*) => { $crate::log::write($crate::log::Level::Error, &format!($($t)*)) } }
#[macro_export]
macro_rules! log_warn { ($($t:tt)*) => { $crate::log::write($crate::log::Level::Warn, &format!($($t)*)) } }
#[macro_export]
macro_rules! log_info { ($($t:tt)*) => { $crate::log::write($crate::log::Level::Info, &format!($($t)*)) } }
#[macro_export]
macro_rules! log_debug { ($($t:tt)*) => { $crate::log::write($crate::log::Level::Debug, &format!($($t)*)) } }
