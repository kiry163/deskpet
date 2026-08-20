//! 平台分发层：按目标平台导出统一的窗口 / 光标 / 退出接口。
#![allow(dead_code)]

#[cfg(windows)]
pub use crate::win32::{cursor_pos, post_quit, PetWindow};
#[cfg(target_os = "macos")]
pub use crate::macos::{cursor_pos, post_quit, PetWindow};
