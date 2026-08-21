//! deskpet 桌宠 —— 原生实现（Windows Win32 / macOS AppKit），libvpx 静态链接，单文件。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[macro_use]
mod log;
mod app;
mod assets;
mod autostart;
mod clip;
mod config;
mod control;
mod db;
mod gfx;
mod import;
mod menu;
mod monitor;
mod pet;
mod platform;
mod single_instance;
mod state;
mod tray;
mod vpx;
mod webm;
#[cfg(windows)]
mod win32;
#[cfg(target_os = "macos")]
mod macos;

fn main() {
    log::init();
    log_info!(
        "deskpet {} 启动 (os={})",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS
    );

    // 单实例：已有实例运行则直接退出（重复双击/自启竞态）
    let _lock = match single_instance::acquire() {
        Some(l) => l,
        None => {
            log_info!("已有 deskpet 实例在运行，本实例退出");
            return;
        }
    };

    #[cfg(windows)]
    unsafe {
        // 感知 DPI（物理像素坐标，避免缩放失真）
        windows_sys::Win32::UI::HiDpi::SetProcessDpiAwarenessContext(-4isize as _); // PER_MONITOR_AWARE_V2
    }
    state::init_random();

    // 本地 HTTP 控制服务（管理前端 + JSON API）：命令通道 → 主线程 tick 排空
    let (api_tx, api_rx) = std::sync::mpsc::channel();
    let mut app = app::App::new(api_rx);
    let assets_root =
        crate::assets::resolve_assets_dir(app.cfg.sys.assets_dir.as_deref(), &app.cfg.dir);
    let port = app.cfg.sys.console_port.unwrap_or(18686);
    app.console = control::ControlServer::start(api_tx, app.cfg.dir.clone(), assets_root, port);
    if app.console.is_none() {
        log_error!("控制服务启动失败（端口绑定失败）");
    }

    #[cfg(windows)]
    {
        // 回调绑定
        win32::set_global_callback(&mut app);
        // 托盘图标：从当前帧生成
        let icon = generate_icon(&mut app);
        let mut tray = tray::Tray::new();
        if !tray.add(app.primary_hwnd(), icon, "deskpet 桌宠") {
            log_warn!("托盘图标添加失败");
        }
        log_info!("桌宠窗口已创建，进入消息循环");
        // 消息循环
        let _ = win32::message_loop();
        app.save_position();
        tray.remove(app.primary_hwnd());
    }

    #[cfg(target_os = "macos")]
    {
        // AppKit 主循环（内部创建窗口/状态栏/定时器，退出时 save_position 已由 quit_all 完成）
        macos::run(&mut app);
    }
}

#[cfg(windows)]
mod win32_main {
    use super::app;
    use std::ffi::c_void;

    use windows_sys::Win32::{
        Graphics::Gdi::{
            BITMAPINFO, BITMAPINFOHEADER, BI_RGB, CreateCompatibleDC, CreateDIBSection,
            DeleteDC, DeleteObject, DIB_RGB_COLORS,
        },
        UI::WindowsAndMessaging::{CreateIconIndirect, ICONINFO},
    };

    /// 从当前渲染帧生成 32×32 托盘图标（CreateIconIndirect）。
    pub fn generate_icon(app: &mut app::App) -> isize {
        let size = 32i32;
        let (dw, dh) = (size as usize, size as usize);

        // 取当前帧 BGRA 缩略（从 render_buf）；无桌宠（未导入素材）时用默认圆形图标
        let (sw, sh, src): (usize, usize, Vec<u8>) = match app.pet.as_mut() {
            Some(p) => (
                p.render_buf.len() / ((crate::clip::H + 30) * 4),
                crate::clip::H + 30,
                p.render_buf.clone(),
            ),
            None => (32, 32, default_icon_bgra()),
        };
        let thumb = make_thumb(&src, sw, sh, dw, dh);

        let hdc = unsafe { CreateCompatibleDC(std::ptr::null_mut()) };
        if hdc.is_null() {
            return 0;
        }

        // 彩色 32bpp 位图
        let mut cbmi: BITMAPINFO = unsafe { std::mem::zeroed() };
        cbmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        cbmi.bmiHeader.biWidth = size;
        cbmi.bmiHeader.biHeight = -size;
        cbmi.bmiHeader.biPlanes = 1;
        cbmi.bmiHeader.biBitCount = 32;
        cbmi.bmiHeader.biCompression = BI_RGB;
        let mut color_bits: *mut c_void = std::ptr::null_mut();
        let color_bmp = unsafe {
            CreateDIBSection(hdc, &cbmi, DIB_RGB_COLORS, &mut color_bits, std::ptr::null_mut(), 0)
        };

        // 1bpp mask 位图
        let mut mbmi: BITMAPINFO = unsafe { std::mem::zeroed() };
        mbmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        mbmi.bmiHeader.biWidth = size;
        mbmi.bmiHeader.biHeight = -size;
        mbmi.bmiHeader.biPlanes = 1;
        mbmi.bmiHeader.biBitCount = 1;
        let mut mask_bits: *mut c_void = std::ptr::null_mut();
        let mask_bmp = unsafe {
            CreateDIBSection(hdc, &mbmi, DIB_RGB_COLORS, &mut mask_bits, std::ptr::null_mut(), 0)
        };

        if color_bmp.is_null() || mask_bmp.is_null() {
            if !color_bmp.is_null() {
                unsafe { DeleteObject(color_bmp) };
            }
            if !mask_bmp.is_null() {
                unsafe { DeleteObject(mask_bmp) };
            }
            unsafe { DeleteDC(hdc) };
            return 0;
        }

        // 填充彩色位图（BGRA premultiplied）
        unsafe {
            let cb = std::slice::from_raw_parts_mut(color_bits as *mut u8, dw * dh * 4);
            for (i, px) in thumb.iter().enumerate() {
                cb[i] = *px;
            }
            // 填充 mask：alpha>0 → 位 0（不透明）
            let stride = ((size + 31) / 32 * 4) as usize;
            let mb = std::slice::from_raw_parts_mut(mask_bits as *mut u8, stride * dh);
            for y in 0..dh {
                for x in 0..dw {
                    let a = thumb[(y * dw + x) * 4 + 3];
                    if a >= 128 {
                        let byte = y * stride + x / 8;
                        mb[byte] |= 0x80 >> (x % 8);
                    }
                }
            }
        }

        let info = ICONINFO {
            fIcon: 1,
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: mask_bmp,
            hbmColor: color_bmp,
        };
        let icon = unsafe { CreateIconIndirect(&info) };

        unsafe {
            DeleteObject(color_bmp);
            DeleteObject(mask_bmp);
            DeleteDC(hdc);
        }
        icon as isize
    }

    /// 无素材时的默认托盘图标（32×32 圆形，BGRA）。
    fn default_icon_bgra() -> Vec<u8> {
        let mut buf = vec![0u8; 32 * 32 * 4];
        for y in 0..32 {
            for x in 0..32 {
                let dx = x as f64 - 15.5;
                let dy = y as f64 - 15.5;
                if dx * dx + dy * dy <= 14.0 * 14.0 {
                    let i = (y * 32 + x) * 4;
                    buf[i] = 0x7f;
                    buf[i + 1] = 0x9c;
                    buf[i + 2] = 0xff;
                    buf[i + 3] = 255;
                }
            }
        }
        buf
    }

    /// 双线性近似缩略图（BGRA）。
    fn make_thumb(src: &[u8], sw: usize, sh: usize, dw: usize, dh: usize) -> Vec<u8> {
        let mut out = vec![0u8; dw * dh * 4];
        for y in 0..dh {
            let sy = (y as u64 * sh as u64 / dh as u64) as usize;
            for x in 0..dw {
                let sx = (x as u64 * sw as u64 / dw as u64) as usize;
                let s = sy * sw * 4 + sx * 4;
                let d = y * dw * 4 + x * 4;
                out[d] = src[s];
                out[d + 1] = src[s + 1];
                out[d + 2] = src[s + 2];
                out[d + 3] = src[s + 3];
            }
        }
        out
    }
}

#[cfg(windows)]
use win32_main::generate_icon;
