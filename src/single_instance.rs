//! 单实例锁：保证同一时刻只有一个桌宠进程。
//!
//! Windows / macOS 统一用 `std::fs::File::try_lock`（advisory 文件锁，跨进程有效，
//! 随进程退出自动释放，崩溃不残留）。锁文件位于配置目录，进程持有直到退出。
#![allow(dead_code)]

use std::fs::{File, OpenOptions};
use std::path::PathBuf;

/// 持有锁的句柄（保持存活直到进程退出）。
pub struct InstanceLock {
    _file: File,
}

/// 尝试获取单实例锁。失败 = 已有实例在运行。
pub fn acquire() -> Option<InstanceLock> {
    let path = lock_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let f = OpenOptions::new().create(true).write(true).open(&path).ok()?;
    if f.try_lock().is_err() {
        return None;
    }
    Some(InstanceLock { _file: f })
}

/// 锁文件：`<配置目录>/deskpet/instance.lock`（与日志同根）。
fn lock_path() -> PathBuf {
    base_dir().join("deskpet").join("instance.lock")
}

/// 平台配置根目录（与 config.rs / log.rs 一致）。
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
