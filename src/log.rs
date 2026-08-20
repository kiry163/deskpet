//! 轻量日志：仅终端输出（不写文件）。
//!
//! - Windows release 默认无控制台（`windows_subsystem = "windows"`）；传 `--console`
//!   参数时 `AttachConsole` 附加父终端，用 `WriteConsoleW` 输出 Unicode 日志；
//! - Windows debug 构建为 console 子系统，直接 `eprintln` 输出；
//! - macOS 直接 `eprintln` 输出（终端运行可见）；
//! - 级别过滤：环境变量 `DESKPET_LOG`（off|error|warn|info|debug，默认 info）。
#![allow(dead_code)]

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

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

#[cfg(windows)]
static CONSOLE: AtomicBool = AtomicBool::new(false);

/// 初始化日志。`attach_console`：Windows 下是否尝试附加父终端（`--console` 参数）。
pub fn init(attach_console: bool) {
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
    #[cfg(windows)]
    if attach_console {
        use windows_sys::Win32::System::Console::AttachConsole;
        unsafe {
            if AttachConsole(windows_sys::Win32::System::Console::ATTACH_PARENT_PROCESS) != 0 {
                CONSOLE.store(true, Ordering::Relaxed);
                write(Level::Info, "已附加父终端，日志输出到控制台");
            }
        }
    }
}

/// 当前级别是否启用。
pub fn enabled(lv: Level) -> bool {
    (lv as u8) <= LEVEL.load(Ordering::Relaxed)
}

/// 写一条日志。
pub fn write(lv: Level, msg: &str) {
    if !enabled(lv) {
        return;
    }
    let line = format!("[{}] [{:<5}] {}", timestamp(), lv.as_str(), msg);
    #[cfg(windows)]
    {
        if CONSOLE.load(Ordering::Relaxed) {
            write_console(&line);
            return;
        }
    }
    eprintln!("{}", line);
}

#[cfg(windows)]
fn write_console(line: &str) {
    use windows_sys::Win32::{
        Foundation::INVALID_HANDLE_VALUE,
        System::Console::{GetStdHandle, WriteConsoleW, STD_OUTPUT_HANDLE},
    };
    let handle = unsafe { GetStdHandle(STD_OUTPUT_HANDLE) };
    if handle.is_null() || handle == INVALID_HANDLE_VALUE {
        return;
    }
    let mut line = line.to_string();
    line.push('\n');
    let wide: Vec<u16> = line.encode_utf16().collect();
    let mut written: u32 = 0;
    unsafe {
        WriteConsoleW(handle, wide.as_ptr(), wide.len() as u32, &mut written, std::ptr::null());
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
