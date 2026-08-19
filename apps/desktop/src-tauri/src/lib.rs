// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::{TrayIcon, TrayIconBuilder};
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_autostart::MacosLauncher;

// ---------------- 配置 ----------------

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct PetConfig {
    pub scale: f64,
    pub no_move: bool,
    pub always_on_top: bool,
    pub autostart: bool,
    pub facing_right: bool,
    pub x: Option<f64>,
    pub y: Option<f64>,
}

impl Default for PetConfig {
    fn default() -> Self {
        PetConfig {
            scale: 0.72,
            no_move: false,
            always_on_top: true,
            autostart: false,
            facing_right: false,
            x: None,
            y: None,
        }
    }
}

fn config_path(app: &AppHandle) -> std::path::PathBuf {
    app.path()
        .app_config_dir()
        .expect("app config dir")
        .join("config.json")
}

fn load_config(app: &AppHandle) -> PetConfig {
    let path = config_path(app);
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_config(app: &AppHandle, cfg: &PetConfig) {
    if let Ok(json) = serde_json::to_string_pretty(cfg) {
        let path = config_path(app);
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(path, json);
    }
}

// ---------------- 命令 ----------------

/// 素材配置（前端用它构造 convertFileSrc 访问素材帧与 meta.json）。
#[tauri::command]
fn get_assets_config() -> serde_json::Value {
    let dir = std::env::var("DESKPET_ASSETS_DIR").unwrap_or_else(|_| {
        "/Users/kiry/code/deskpet/assets/frames".to_string()
    });
    serde_json::json!({ "framesDir": dir })
}

/// 工作区（主屏）逻辑尺寸与物理位置：overlay 窗口铺满工作区，桌宠在其中自由移动。
#[tauri::command]
fn get_work_area(app: AppHandle) -> serde_json::Value {
    match app.primary_monitor().ok().flatten() {
        Some(m) => {
            let size = m.size(); // 物理像素
            let pos = m.position();
            let sf = m.scale_factor();
            serde_json::json!({
                "width": size.width as f64 / sf,
                "height": size.height as f64 / sf,
                "x": pos.x,
                "y": pos.y,
                "scaleFactor": sf,
            })
        }
        None => serde_json::json!({
            "width": 1280.0, "height": 800.0, "x": 0, "y": 0, "scaleFactor": 1.0,
        }),
    }
}

#[tauri::command]
fn get_config(app: AppHandle) -> PetConfig {
    load_config(&app)
}

#[tauri::command]
fn save_config_cmd(app: AppHandle, cfg: PetConfig) {
    save_config(&app, &cfg);
}

/// 可交互区域（窗口内逻辑坐标），由前端每帧上报；穿透轮询线程据此切换点击穿透。
#[derive(Clone, Default)]
pub struct HitRegion(pub Arc<Mutex<Option<(f64, f64, f64, f64)>>>);

#[tauri::command]
fn update_hit_region(state: State<'_, HitRegion>, x: f64, y: f64, w: f64, h: f64) {
    *state.0.lock().unwrap() = Some((x, y, w, h));
}

// ---------------- 平台光标位置 ----------------

#[cfg(target_os = "macos")]
fn cursor_pos() -> Option<(f64, f64)> {
    use core_graphics::event::CGEvent;
    use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
    let source = CGEventSource::new(CGEventSourceStateID::CombinedSessionState).ok()?;
    let ev = CGEvent::new(source).ok()?;
    // CGEvent.location 返回 Quartz 全局坐标（点/逻辑单位，原点左上，y 向下），
    // 与窗口的逻辑坐标同源，无需转换。
    let p = ev.location();
    Some((p.x, p.y))
}

#[cfg(windows)]
fn cursor_pos() -> Option<(f64, f64)> {
    use windows_sys::Win32::Foundation::POINT;
    use windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos;
    let mut pt = POINT { x: 0, y: 0 };
    // SAFETY: GetCursorPos 只写 pt
    unsafe { GetCursorPos(&mut pt) };
    Some((pt.x as f64, pt.y as f64))
}

// ---------------- 穿透轮询 ----------------

fn start_clickthrough(app: AppHandle, region: Arc<Mutex<Option<(f64, f64, f64, f64)>>>) {
    std::thread::spawn(move || {
        let mut last_ignore: Option<bool> = None;
        loop {
            std::thread::sleep(Duration::from_millis(33)); // ~30Hz
            let Some((cx, cy)) = cursor_pos() else { continue };
            let Some(win) = app.get_webview_window("main") else { continue };
            let Ok(pos) = win.outer_position() else { continue };
            let sf = win.scale_factor().unwrap_or(1.0);
            let reg = region.lock().unwrap().clone();
            // outer_position 是物理像素，转逻辑（点）坐标与光标/CGEvent 同源比较
            let win_x = pos.x as f64 / sf;
            let win_y = pos.y as f64 / sf;
            let inside = reg
                .as_ref()
                .map(|(x, y, w, h)| {
                    cx >= win_x + x
                        && cx <= win_x + x + w
                        && cy >= win_y + y
                        && cy <= win_y + y + h
                })
                .unwrap_or(false);
            let should_ignore = !inside;
            if last_ignore != Some(should_ignore) {
                let _ = win.set_ignore_cursor_events(should_ignore);
                last_ignore = Some(should_ignore);
            }
        }
    });
}

// ---------------- 托盘与菜单 ----------------

/// 托盘句柄（菜单勾选状态刷新用）。
pub struct TrayHandle(pub Mutex<Option<TrayIcon>>);

fn build_menu(app: &AppHandle, cfg: &PetConfig) -> tauri::Result<Menu<tauri::Wry>> {
    let show_hide = MenuItem::with_id(app, "toggle_visible", "隐藏/显示", true, None::<&str>)?;
    let topmost = CheckMenuItem::with_id(app, "toggle_topmost", "置顶", true, cfg.always_on_top, None::<&str>)?;
    let no_move = CheckMenuItem::with_id(app, "toggle_no_move", "不移动", true, cfg.no_move, None::<&str>)?;
    let autostart = CheckMenuItem::with_id(app, "toggle_autostart", "开机自启", true, cfg.autostart, None::<&str>)?;

    let scale_sub = Submenu::with_items(
        app,
        "大小",
        true,
        &[
            &CheckMenuItem::with_id(app, "scale_050", "50%", true, cfg.scale == 0.5, None::<&str>)?,
            &CheckMenuItem::with_id(app, "scale_072", "72%", true, cfg.scale == 0.72, None::<&str>)?,
            &CheckMenuItem::with_id(app, "scale_085", "85%", true, cfg.scale == 0.85, None::<&str>)?,
            &CheckMenuItem::with_id(app, "scale_100", "100%", true, cfg.scale == 1.0, None::<&str>)?,
        ],
    )?;

    let sep = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;

    Menu::with_items(
        app,
        &[&show_hide, &topmost, &no_move, &autostart, &scale_sub, &sep, &quit],
    )
}

fn refresh_tray_menu(app: &AppHandle, cfg: &PetConfig) {
    let Ok(menu) = build_menu(app, cfg) else { return };
    if let Some(tray) = app.state::<TrayHandle>().0.lock().unwrap().as_ref() {
        let _ = tray.set_menu(Some(menu));
    }
}

fn apply_scale(app: &AppHandle, scale: f64) {
    let _ = app.emit("pet-scale", scale);
    let mut cfg = load_config(app);
    cfg.scale = scale;
    save_config(app, &cfg);
    refresh_tray_menu(app, &cfg);
}

fn handle_menu(app: &AppHandle, id: &str) {
    let win = app.get_webview_window("main");
    match id {
        "toggle_visible" => {
            if let Some(w) = &win {
                if w.is_visible().unwrap_or(false) {
                    let _ = w.hide();
                } else {
                    let _ = w.show();
                }
            }
        }
        "toggle_topmost" => {
            let mut cfg = load_config(app);
            cfg.always_on_top = !cfg.always_on_top;
            if let Some(w) = &win {
                let _ = w.set_always_on_top(cfg.always_on_top);
            }
            save_config(app, &cfg);
            refresh_tray_menu(app, &cfg);
        }
        "toggle_no_move" => {
            let mut cfg = load_config(app);
            cfg.no_move = !cfg.no_move;
            save_config(app, &cfg);
            let _ = app.emit("pet-no-move", cfg.no_move);
            refresh_tray_menu(app, &cfg);
        }
        "toggle_autostart" => {
            use tauri_plugin_autostart::ManagerExt;
            let mut cfg = load_config(app);
            cfg.autostart = !cfg.autostart;
            let autostart = app.autolaunch();
            if cfg.autostart {
                let _ = autostart.enable();
            } else {
                let _ = autostart.disable();
            }
            save_config(app, &cfg);
            refresh_tray_menu(app, &cfg);
        }
        "scale_050" => apply_scale(app, 0.5),
        "scale_072" => apply_scale(app, 0.72),
        "scale_085" => apply_scale(app, 0.85),
        "scale_100" => apply_scale(app, 1.0),
        "quit" => {
            if let Some(w) = &win {
                let pos = w.outer_position().ok();
                let mut cfg = load_config(app);
                cfg.x = pos.map(|p| p.x as f64);
                cfg.y = pos.map(|p| p.y as f64);
                save_config(app, &cfg);
            }
            app.exit(0);
        }
        _ => {}
    }
}

// ---------------- 入口 ----------------

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec![]),
        ))
        .manage(HitRegion::default())
        .manage(TrayHandle(Mutex::new(None)))
        .invoke_handler(tauri::generate_handler![
            get_assets_config,
            get_work_area,
            get_config,
            save_config_cmd,
            update_hit_region,
        ])
        .setup(|app| {
            let cfg = load_config(app.handle());
            // overlay：窗口铺满主屏（物理尺寸+位置），桌宠在窗口内自由移动
            if let (Some(win), Some(mon)) = (
                app.get_webview_window("main"),
                app.primary_monitor().ok().flatten(),
            ) {
                let _ = win.set_size(*mon.size());
                let _ = win.set_position(*mon.position());
                if cfg.always_on_top {
                    let _ = win.set_always_on_top(true);
                }
            }
            // 托盘
            let menu = build_menu(app.handle(), &cfg)?;
            let tray = TrayIconBuilder::with_id("main")
                .icon(app.default_window_icon().unwrap().clone())
                .menu(&menu)
                .show_menu_on_left_click(true)
                .on_menu_event(|app, event| handle_menu(app, event.id().as_ref()))
                .build(app)?;
            *app.state::<TrayHandle>().0.lock().unwrap() = Some(tray);
            // 穿透轮询
            let region = app.state::<HitRegion>();
            start_clickthrough(app.handle().clone(), region.0.clone());
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
