//! 屏幕工作区查询。
//! Windows：主屏工作区（物理像素）；macOS：主屏可见区（点，左上原点）。
#![allow(dead_code)]

#[cfg(windows)]
mod win32_monitor {
    use windows_sys::Win32::UI::WindowsAndMessaging::SystemParametersInfoW;
    use windows_sys::Win32::Foundation::RECT;

    pub const SPI_GETWORKAREA: u32 = 0x0030;

    /// 主屏工作区 (left, top, right, bottom)。
    pub fn primary_work_area() -> (i32, i32, i32, i32) {
        let mut r: RECT = unsafe { std::mem::zeroed() };
        unsafe {
            SystemParametersInfoW(SPI_GETWORKAREA, 0, &mut r as *mut _ as *mut _, 0);
        }
        let out = (r.left, r.top, r.right, r.bottom);
        log_debug!("主屏工作区: {:?}", out);
        out
    }
}

#[cfg(target_os = "macos")]
mod macos_monitor {
    use objc2_app_kit::NSScreen;
    use objc2_foundation::MainThreadMarker;

    /// 主屏工作区 (left, top, right, bottom)，左上原点（点）。
    /// NSScreen.visibleFrame 为左下原点，转换到左上原点以便与 win32 坐标语义一致。
    pub fn primary_work_area() -> (i32, i32, i32, i32) {
        let Some(mtm) = MainThreadMarker::new() else { return (0, 0, 1, 1) };
        let Some(screen) = NSScreen::mainScreen(mtm) else { return (0, 0, 1, 1) };
        let frame = screen.frame();
        let vis = screen.visibleFrame();
        let h = frame.size.height;
        let l = vis.origin.x;
        let t = h - vis.origin.y - vis.size.height;
        let r = vis.origin.x + vis.size.width;
        let b = h - vis.origin.y;
        let out = (l as i32, t as i32, r as i32, b as i32);
        log_debug!("主屏工作区: {:?}", out);
        out
    }
}

#[cfg(windows)]
pub use win32_monitor::primary_work_area;
#[cfg(target_os = "macos")]
pub use macos_monitor::primary_work_area;
