//! deskpet 桌宠 —— 原生实现（Windows Win32 / macOS AppKit），libvpx 静态链接，单文件。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[macro_use]
mod log;
mod app;
mod assets;
mod autostart;
mod behavior;
mod clip;
mod config;
mod control;
mod convert;
mod db;
mod gfx;
mod import;
mod menu;
mod monitor;
mod pet;
mod platform;
mod single_instance;
mod state;
mod thumb;
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
    let mut app = app::App::new(api_rx, api_tx.clone());
    let assets_root =
        crate::assets::resolve_assets_dir(app.cfg.sys.assets_dir.as_deref(), &app.cfg.dir);
    let port = app.cfg.sys.console_port.unwrap_or(18686);
    app.console = control::ControlServer::start(api_tx, app.cfg.dir.clone(), assets_root, port);
    if app.console.is_none() {
        log_error!("控制服务启动失败（端口绑定失败）");
    }

    // 补生成：为历史导入（尚缺全身照）的宠物自动取帧，保证控制台展示全身照
    app.backfill_full_body_images();

    // 无素材时启动自动打开控制台，引导用户导入素材包
    if app.pet.is_none() {
        log_info!("未找到桌宠素材，自动打开控制台引导导入");
        app.open_console();
    }

    #[cfg(windows)]
    {
        // 回调绑定
        win32::set_global_callback(&mut app);
        // 托盘图标：固定使用嵌入资源（resources/deskpet.ico，ID 1）
        let icon = win32_main::load_tray_icon();
        let mut tray = tray::Tray::new();
        if !tray.add(app.primary_hwnd(), icon, "DeskPet") {
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
    use windows_sys::Win32::{
        System::LibraryLoader::GetModuleHandleW,
        UI::WindowsAndMessaging::{LoadImageW, IMAGE_ICON, LR_DEFAULTCOLOR},
    };

    /// 加载嵌入资源图标（resources/deskpet.ico，资源 ID 1）作为托盘图标（HICON）。
    pub fn load_tray_icon() -> isize {
        unsafe {
            let hinst = GetModuleHandleW(std::ptr::null());
            // 资源 ID 1（resources/deskpet.rc: `1 ICON "deskpet.ico"`）
            LoadImageW(hinst, 1 as *const u16, IMAGE_ICON, 32, 32, LR_DEFAULTCOLOR) as isize
        }
    }
}
