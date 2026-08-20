//! 桌宠应用：单只桌宠（窗口/状态机/交互）+ 托盘 + 菜单分发（平台无关核心）。
//! Windows 消息桥接（WindowCallback）在 #[cfg(windows)] 下实现；macOS 由 macos.rs 直接调用。
#![allow(non_snake_case, dead_code)]

use crate::config::Config;
use crate::pet::{self, Pet};

pub const WM_TRAY: u32 = 0x0400 + 100; // win32 托盘回调消息（与 tray.rs 一致）

pub struct App {
    pub pet: Option<Pet>,
    pub cfg: Config,
    pub quitting: bool,
}

impl App {
    pub fn new() -> App {
        let cfg = Config::load();
        let role = match crate::assets::load_builtin() {
            Some(r) => std::rc::Rc::new(r),
            // 极端情况：素材解析全部失败
            None => {
                return App {
                    pet: None,
                    cfg,
                    quitting: false,
                };
            }
        };
        let mut app = App {
            pet: None,
            cfg,
            quitting: false,
        };
        if let Some(win) = crate::platform::PetWindow::create(app.cfg.pet.always_on_top) {
            let mut pet = Pet::new(win, &role, &app.cfg.pet);
            pet.restore_position(&app.cfg.pet);
            app.pet = Some(pet);
        }
        app
    }

    /// 保存宠物位置与设置。
    pub fn save_position(&mut self) {
        if let Some(pet) = &mut self.pet {
            pet.save_position(&mut self.cfg.pet);
            self.cfg.save();
        }
    }

    pub fn quit_all(&mut self) {
        self.quitting = true;
        self.save_position();
        crate::platform::post_quit();
    }

    pub fn toggle_visible(&mut self) {
        if let Some(pet) = &mut self.pet {
            pet.toggle_visible();
        }
    }

    #[cfg(windows)]
    pub fn primary_hwnd(&self) -> windows_sys::Win32::Foundation::HWND {
        self.pet.as_ref().map(|p| p.win.hwnd).unwrap_or(std::ptr::null_mut())
    }

    /// 处理托盘菜单命令。
    pub fn handle_tray_command(&mut self, cmd: usize) {
        match cmd {
            crate::tray::TRAY_TOGGLE_VISIBLE => self.toggle_visible(),
            crate::tray::TRAY_AUTOSTART => {
                let on = !crate::autostart::is_enabled();
                crate::autostart::set_enabled(on);
            }
            crate::tray::TRAY_QUIT => self.quit_all(),
            _ => {}
        }
    }

    // ---------------- 平台无关交互入口（win32 消息桥 / macOS 视图回调共用） ----------------

    pub fn on_pet_press(&mut self, cx: i32, cy: i32) {
        if let Some(pet) = &mut self.pet {
            pet.on_press(cx, cy);
        }
    }

    pub fn on_pet_drag(&mut self) {
        if let Some(pet) = &mut self.pet {
            pet.on_drag_move();
        }
    }

    pub fn on_pet_release(&mut self) {
        if let Some(pet) = &mut self.pet {
            pet.on_release();
        }
        // 拖拽/点击后保存位置
        self.save_position();
    }

    pub fn on_pet_tick(&mut self) {
        if let Some(pet) = &mut self.pet {
            pet.on_tick();
        }
    }

    /// 处理宠物右键菜单命令（自启全局命令 + 宠物自身命令）。
    pub fn handle_command(&mut self, cmd: usize) {
        if cmd == pet::MID_AUTOSTART {
            let on = !crate::autostart::is_enabled();
            crate::autostart::set_enabled(on);
        } else if let Some(pet) = &mut self.pet {
            pet.apply_command(cmd);
            self.save_position();
        }
    }
}

// ---------------- Windows 消息桥 ----------------

#[cfg(windows)]
mod win32_bridge {
    use super::App;
    use crate::menu::MenuEntry;
    use crate::win32::{self, FRAME_TIMER, WindowCallback};
    use windows_sys::Win32::{
        Foundation::{HWND, LRESULT, LPARAM, WPARAM},
        UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture},
        UI::WindowsAndMessaging::{
            GetWindowLongPtrW, LoadCursorW, PostMessageW, SetCursor, SetForegroundWindow,
            SetWindowLongPtrW, GWL_EXSTYLE, IDC_HAND, WM_LBUTTONDOWN, WM_LBUTTONUP,
            WM_MOUSEMOVE, WM_NCHITTEST, WM_NULL, WM_SETCURSOR, WM_TIMER, WS_EX_NOACTIVATE,
            HTCLIENT, HTTRANSPARENT,
        },
    };

    impl WindowCallback for App {
        fn on_wnd_message(&mut self, hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
            match msg {
                WM_NCHITTEST => {
                    let gx = (lparam & 0xFFFF) as i16 as i32;
                    let gy = ((lparam >> 16) & 0xFFFF) as i16 as i32;
                    let hit = self.pet.as_ref().map(|p| p.hit_at(gx, gy)).unwrap_or(false);
                    Some(if hit { HTCLIENT as LRESULT } else { HTTRANSPARENT as LRESULT })
                }
                WM_LBUTTONDOWN => {
                    // 捕获鼠标：快速拖拽时鼠标移出窗口仍能收到事件
                    unsafe { SetCapture(hwnd) };
                    let (cx, cy) = client_pos(lparam);
                    self.on_pet_press(cx, cy);
                    Some(0)
                }
                WM_MOUSEMOVE => {
                    self.on_pet_drag();
                    Some(0)
                }
                WM_LBUTTONUP => {
                    self.on_pet_release();
                    unsafe { ReleaseCapture() };
                    Some(0)
                }
                // 光标悬停在桌宠身上（不透明像素）→ 手型，提示可交互。
                // 移出到透明区域时 WM_NCHITTEST 返回 HTTRANSPARENT，光标由下层窗口恢复箭头。
                WM_SETCURSOR => {
                    let n_hit = (lparam & 0xFFFF) as u32;
                    if n_hit == HTCLIENT {
                        let (sx, sy) = win32::cursor_pos();
                        let hit = self.pet.as_ref().map(|p| p.hit_at(sx, sy)).unwrap_or(false);
                        if hit {
                            let cursor = unsafe { LoadCursorW(std::ptr::null_mut(), IDC_HAND) };
                            if !cursor.is_null() {
                                unsafe { SetCursor(cursor) };
                            }
                            return Some(0);
                        }
                    }
                    None
                }
                WM_TIMER if (wparam as usize) == FRAME_TIMER => {
                    self.on_pet_tick();
                    Some(0)
                }
                super::WM_TRAY => {
                    // Shell_NotifyIconW 回调：lParam 低字 = 触发鼠标消息
                    match (lparam & 0xFFFF) as u32 {
                        0x0202 => {
                            // WM_LBUTTONUP：左键单击切换显示/隐藏
                            self.toggle_visible();
                        }
                        0x0205 => {
                            // WM_RBUTTONUP：右键弹出完整菜单（桌宠菜单 + 托盘项）
                            self.show_tray_menu();
                        }
                        _ => {}
                    }
                    Some(0)
                }
                _ => None,
            }
        }
    }

    impl App {
        /// 托盘右键菜单：桌宠菜单（角落/置顶/不移动/自启/大小）+ 显示隐藏/退出。
        fn show_tray_menu(&mut self) {
            let hwnd = self.primary_hwnd();
            if hwnd.is_null() {
                return;
            }
            let mut items = self
                .pet
                .as_ref()
                .map(|p| p.context_menu_items())
                .unwrap_or_default();
            items.push(MenuEntry::separator());
            items.push(MenuEntry::item(crate::tray::TRAY_TOGGLE_VISIBLE, "显示/隐藏"));
            items.push(MenuEntry::item(crate::tray::TRAY_QUIT, "退出"));
            let (sx, sy) = crate::win32::cursor_pos();
            // NOACTIVATE 窗口弹菜单的经典修复：
            // 1) 临时移除 WS_EX_NOACTIVATE，否则 SetForegroundWindow 无效（窗口不可激活）；
            // 2) SetForegroundWindow 让本窗口成为前台——TrackPopupMenu 的菜单才能成为前台，
            //    否则点击菜单外区域时菜单没有失活事件，不会自动关闭；
            // 3) 菜单返回（已关闭）后恢复 WS_EX_NOACTIVATE，并 PostMessage(WM_NULL)
            //    让系统正确回收焦点。
            let ex = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) };
            unsafe {
                SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex & !(WS_EX_NOACTIVATE as isize));
                SetForegroundWindow(hwnd);
            }
            let cmd = crate::win32::show_menu_blocking(hwnd, &items, sx, sy);
            unsafe {
                SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex);
                PostMessageW(hwnd, WM_NULL, 0, 0);
            }
            if cmd != 0 {
                if cmd >= crate::tray::TRAY_TOGGLE_VISIBLE {
                    self.handle_tray_command(cmd);
                } else {
                    self.handle_command(cmd);
                }
            }
        }
    }

    fn client_pos(lparam: LPARAM) -> (i32, i32) {
        let x = (lparam & 0xFFFF) as i16 as i32;
        let y = ((lparam >> 16) & 0xFFFF) as i16 as i32;
        (x, y)
    }
}
