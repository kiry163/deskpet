//! 系统托盘。
//! Windows：Shell_NotifyIconW 通知区图标；macOS：NSStatusItem（见 macos.rs）。
//! 完整右键菜单（桌宠菜单 + 托盘项）由各平台后端组装：win32 见 app.rs，macOS 见 macos.rs。
#![allow(dead_code)]

// 托盘菜单命令
pub const TRAY_TOGGLE_VISIBLE: usize = 1001;
pub const TRAY_AUTOSTART: usize = 1002;
pub const TRAY_QUIT: usize = 1003;

// ---------------- Windows ----------------

#[cfg(windows)]
mod win32_tray {
    use windows_sys::Win32::{
        Foundation::HWND,
        UI::Shell::{
            NOTIFYICONDATAW, Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE,
        },
    };

    use crate::app::WM_TRAY;

    pub struct Tray {
        added: bool,
    }

    impl Tray {
        pub fn new() -> Tray {
            Tray { added: false }
        }

        /// 添加托盘图标。icon 为 HICON。返回是否成功。
        pub fn add(&mut self, hwnd: HWND, icon: isize, tooltip: &str) -> bool {
            if icon == 0 {
                return false;
            }
            let mut nid: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
            nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
            nid.hWnd = hwnd;
            nid.uID = 1;
            nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
            nid.uCallbackMessage = WM_TRAY;
            nid.hIcon = icon as _;
            let tip: Vec<u16> = tooltip.encode_utf16().chain(std::iter::once(0)).collect();
            for (i, c) in tip.iter().take(127).enumerate() {
                nid.szTip[i] = *c;
            }
            let ok = unsafe { Shell_NotifyIconW(NIM_ADD, &nid) };
            self.added = ok != 0;
            ok != 0
        }

        pub fn remove(&mut self, hwnd: HWND) {
            if !self.added {
                return;
            }
            let mut nid: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
            nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
            nid.hWnd = hwnd;
            nid.uID = 1;
            unsafe { Shell_NotifyIconW(NIM_DELETE, &nid) };
            self.added = false;
        }
    }

    impl Default for Tray {
        fn default() -> Self {
            Tray::new()
        }
    }
}

#[cfg(windows)]
pub use win32_tray::*;
