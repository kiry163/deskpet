//! Win32 原生窗口：透明无边框置顶窗口 + UpdateLayeredWindow 逐像素渲染 + 鼠标穿透。
#![allow(non_snake_case, clippy::too_many_arguments, dead_code)]

use std::ffi::c_void;
use windows_sys::Win32::{
    Foundation::{HINSTANCE, HWND, LRESULT, LPARAM, POINT, RECT, SIZE, WPARAM},
    Graphics::Gdi::{
        BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION, CreateCompatibleDC, CreateDIBSection,
        DeleteDC, DeleteObject, DIB_RGB_COLORS, SelectObject, AC_SRC_ALPHA, AC_SRC_OVER,
    },
    System::LibraryLoader::GetModuleHandleW,
    UI::WindowsAndMessaging::{
        AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu,
        DispatchMessageW, GetCursorPos, GetMessageW, GetWindowRect, HMENU, IsWindowVisible,
        PostQuitMessage, RegisterClassExW, SetTimer, SetWindowPos, ShowWindow, TrackPopupMenu,
        TranslateMessage, UpdateLayeredWindow, CS_HREDRAW, CS_VREDRAW, MF_CHECKED, MF_POPUP,
        MF_SEPARATOR, MF_STRING, MSG, SW_HIDE, SW_SHOW, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
        SWP_NOZORDER, TPM_RETURNCMD, TPM_RIGHTBUTTON, HWND_NOTOPMOST, HWND_TOPMOST,
        WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
    },
};

use crate::menu::MenuEntry;

/// 窗口消息回调（App 实现）。hwnd 用于区分多窗口。
pub trait WindowCallback {
    /// 返回 Some(lresult) 表示已处理；None 走默认处理。
    fn on_wnd_message(
        &mut self,
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> Option<LRESULT>;
}

// 全局回调指针（应用级）。存 Box<&mut dyn WindowCallback> 的瘦指针。
static mut CB: *mut c_void = std::ptr::null_mut();

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let raw = CB as *mut &mut dyn WindowCallback;
    if !raw.is_null() {
        let cb: &mut &mut dyn WindowCallback = unsafe { &mut *raw };
        if let Some(r) = cb.on_wnd_message(hwnd, msg, wparam, lparam) {
            return r;
        }
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

/// 预渲染的说话气泡位图（BGRA premultiplied）。
pub struct BubbleBitmap {
    pub buf: Vec<u8>,
    pub w: usize,
    pub h: usize,
}

pub struct PetWindow {
    pub hwnd: HWND,
    hdc: isize,
    bmp: isize,
    old_bmp: isize,
    pub width: i32,
    pub height: i32,
    bits: *mut u8,
    pub alpha: Vec<u8>,
    /// 说话气泡（say API，present 时合成到顶部）。
    pub bubble: Option<BubbleBitmap>,
}

pub const FRAME_TIMER: usize = 1;

/// 设置窗口消息回调（App 指针）。
pub fn set_global_callback(cb: &mut dyn WindowCallback) {
    let boxed: Box<&mut dyn WindowCallback> = Box::new(cb);
    unsafe { CB = Box::into_raw(boxed) as *mut c_void };
}

impl PetWindow {
    /// 创建透明置顶窗口。
    pub fn create(on_top: bool) -> Option<PetWindow> {
        let instance: HINSTANCE = unsafe { GetModuleHandleW(std::ptr::null()) } as HINSTANCE;
        let class_name: Vec<u16> = "dsh_pet_win\0".encode_utf16().collect();
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wnd_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: instance,
            hIcon: std::ptr::null_mut(),
            hCursor: std::ptr::null_mut(),
            hbrBackground: std::ptr::null_mut(),
            lpszMenuName: std::ptr::null(),
            lpszClassName: class_name.as_ptr(),
            hIconSm: std::ptr::null_mut(),
        };
        unsafe { RegisterClassExW(&wc) };

        let mut ex_style = WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE;
        if on_top {
            ex_style |= WS_EX_TOPMOST;
        }
        let hwnd = unsafe {
            CreateWindowExW(
                ex_style,
                class_name.as_ptr(),
                std::ptr::null(),
                WS_POPUP,
                0,
                0,
                1,
                1,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                instance,
                std::ptr::null_mut(),
            )
        };
        if hwnd.is_null() {
            return None;
        }
        let hdc = unsafe { CreateCompatibleDC(std::ptr::null_mut()) };
        if hdc.is_null() {
            return None;
        }
        let mut pet = PetWindow {
            hwnd,
            hdc: hdc as isize,
            bmp: 0,
            old_bmp: 0,
            width: 1,
            height: 1,
            bits: std::ptr::null_mut(),
            alpha: Vec::new(),
            bubble: None,
        };
        pet.resize(1, 1);
        Some(pet)
    }

    /// 重建位图（缩放档位改变时；尺寸不变时跳过，避免每帧重建）。
    pub fn resize(&mut self, width: i32, height: i32) {
        if width < 1 || height < 1 {
            return;
        }
        if width == self.width && height == self.height {
            return;
        }
        if self.bmp != 0 {
            unsafe {
                SelectObject(self.hdc as _, self.old_bmp as _);
                DeleteObject(self.bmp as _);
            }
            self.bmp = 0;
        }
        let mut bmi: BITMAPINFO = unsafe { std::mem::zeroed() };
        bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bmi.bmiHeader.biWidth = width;
        bmi.bmiHeader.biHeight = -height; // top-down
        bmi.bmiHeader.biPlanes = 1;
        bmi.bmiHeader.biBitCount = 32;
        bmi.bmiHeader.biCompression = BI_RGB;

        let mut bits: *mut c_void = std::ptr::null_mut();
        let bmp = unsafe {
            CreateDIBSection(
                self.hdc as _,
                &bmi,
                DIB_RGB_COLORS,
                &mut bits,
                std::ptr::null_mut(),
                0,
            )
        };
        if bmp.is_null() {
            return;
        }
        let old = unsafe { SelectObject(self.hdc as _, bmp as _) };
        self.bmp = bmp as isize;
        self.old_bmp = old as isize;
        self.bits = bits as *mut u8;
        self.width = width;
        self.height = height;
        self.alpha = vec![0u8; (width * height) as usize];
    }

    pub fn show(&self) {
        unsafe { ShowWindow(self.hwnd, SW_SHOW) };
    }

    pub fn hide(&self) {
        unsafe { ShowWindow(self.hwnd, SW_HIDE) };
    }

    pub fn is_visible(&self) -> bool {
        unsafe { IsWindowVisible(self.hwnd) != 0 }
    }

    pub fn move_to(&self, x: i32, y: i32) {
        unsafe {
            SetWindowPos(
                self.hwnd,
                std::ptr::null_mut(),
                x,
                y,
                0,
                0,
                SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            )
        };
    }

    pub fn set_topmost(&self, on: bool) {
        unsafe {
            SetWindowPos(
                self.hwnd,
                if on { HWND_TOPMOST } else { HWND_NOTOPMOST },
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            )
        };
    }

    pub fn get_rect(&self) -> (i32, i32, i32, i32) {
        let mut r: RECT = unsafe { std::mem::zeroed() };
        unsafe { GetWindowRect(self.hwnd, &mut r) };
        (r.left, r.top, r.right, r.bottom)
    }

    /// 渲染：把 src（sw×sh BGRA）缩放到窗口并 UpdateLayeredWindow。
    pub fn present(&mut self, src: &[u8], sw: usize, sh: usize, mirror: bool) {
        let dw = self.width as usize;
        let dh = self.height as usize;
        if self.bits.is_null() || src.len() < sw * sh * 4 || dw == 0 || dh == 0 {
            return;
        }
        let stride = dw * 4;
        unsafe {
            let dst = std::slice::from_raw_parts_mut(self.bits, stride * dh);
            crate::gfx::scale_bgra(src, sw, sh, dst, dw, dh, stride, mirror);
            // 说话气泡合成到窗口顶部中央（premultiplied alpha 混合）
            if let Some(b) = &self.bubble {
                composite_bubble(dst, dw, dh, b);
            }
            for y in 0..dh {
                let row = y * stride;
                for x in 0..dw {
                    self.alpha[y * dw + x] = dst[row + x * 4 + 3];
                }
            }
        }
        let size = SIZE {
            cx: self.width,
            cy: self.height,
        };
        let pt_src = POINT { x: 0, y: 0 };
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        unsafe {
            // pptDst 传 NULL：保持窗口当前位置不变（否则会把窗口移到 pptDst）
            UpdateLayeredWindow(
                self.hwnd,
                std::ptr::null_mut(),
                std::ptr::null(),
                &size,
                self.hdc as _,
                &pt_src,
                0,
                &blend,
                2, // ULW_ALPHA
            );
        }
    }

    /// 设置说话气泡（say API）：GDI 预渲染为位图，present 时合成。
    pub fn set_bubble(&mut self, text: &str) {
        self.bubble = render_bubble(text);
    }

    pub fn clear_bubble(&mut self) {
        self.bubble = None;
    }

    /// 命中测试：像素 alpha<128 穿透。
    pub fn hit_test_alpha(&self, x: i32, y: i32) -> bool {
        if x < 0 || y < 0 || x >= self.width || y >= self.height {
            return false;
        }
        self.alpha[(y * self.width + x) as usize] >= 128
    }

    pub fn start_frame_timer(&self, interval_ms: u32) {
        unsafe { SetTimer(self.hwnd, FRAME_TIMER, interval_ms, None) };
    }

    pub fn set_frame_timer_interval(&self, interval_ms: u32) {
        unsafe { SetTimer(self.hwnd, FRAME_TIMER, interval_ms, None) };
    }
}

impl Drop for PetWindow {
    fn drop(&mut self) {
        if self.bmp != 0 {
            unsafe {
                SelectObject(self.hdc as _, self.old_bmp as _);
                DeleteObject(self.bmp as _);
            }
        }
        if self.hdc != 0 {
            unsafe { DeleteDC(self.hdc as _) };
        }
    }
}

/// GDI 预渲染说话气泡：圆角白底 + 深色文字 → BGRA premultiplied 位图。
/// 字体优先微软雅黑（中文），回退系统默认。
fn render_bubble(text: &str) -> Option<BubbleBitmap> {
    use windows_sys::Win32::Foundation::{RECT, SIZE};
    use windows_sys::Win32::Graphics::Gdi::{
        CreateCompatibleDC, CreateDIBSection, CreateFontW, CreateSolidBrush, DeleteDC, DeleteObject,
        DrawTextW, GetStockObject, GetTextExtentPoint32W, RoundRect, SelectObject, SetBkMode,
        SetTextColor, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CLEARTYPE_QUALITY, DEFAULT_CHARSET,
        DIB_RGB_COLORS, DT_CENTER, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, FW_NORMAL, TRANSPARENT,
        WHITE_BRUSH,
    };

    let hdc = unsafe { CreateCompatibleDC(std::ptr::null_mut()) };
    if hdc.is_null() {
        return None;
    }
    // 字体：微软雅黑 16px（负值 = 字符高度）
    let face: Vec<u16> = "Microsoft YaHei\0".encode_utf16().collect();
    let font = unsafe {
        CreateFontW(
            -16, 0, 0, 0, FW_NORMAL as i32, 0, 0, 0, DEFAULT_CHARSET as u32, 0, 0,
            CLEARTYPE_QUALITY as u32, 0, face.as_ptr(),
        )
    };
    if font.is_null() {
        unsafe { DeleteDC(hdc) };
        return None;
    }
    let old_font = unsafe { SelectObject(hdc, font as _) };

    // 文本尺寸
    let text_w: Vec<u16> = text.encode_utf16().collect();
    let mut sz: SIZE = unsafe { std::mem::zeroed() };
    unsafe { GetTextExtentPoint32W(hdc, text_w.as_ptr(), text_w.len() as i32, &mut sz) };
    let pad_x = 14i32;
    let pad_y = 7i32;
    let bw = (sz.cx + pad_x * 2).max(40) as usize;
    let bh = (sz.cy + pad_y * 2).max(24) as usize;

    // 32bpp 顶向下 DIB（初始全透明）
    let mut bmi: BITMAPINFO = unsafe { std::mem::zeroed() };
    bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
    bmi.bmiHeader.biWidth = bw as i32;
    bmi.bmiHeader.biHeight = -(bh as i32);
    bmi.bmiHeader.biPlanes = 1;
    bmi.bmiHeader.biBitCount = 32;
    bmi.bmiHeader.biCompression = BI_RGB;
    let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
    let bmp = unsafe {
        CreateDIBSection(hdc, &bmi, DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0)
    };
    if bmp.is_null() {
        unsafe {
            DeleteObject(font as _);
            DeleteDC(hdc);
        }
        return None;
    }
    let old_bmp = unsafe { SelectObject(hdc, bmp as _) };

    // 圆角白底
    let brush = unsafe { CreateSolidBrush(0x00FF_FFFF) };
    if !brush.is_null() {
        unsafe {
            let old_brush = SelectObject(hdc, brush as _);
            RoundRect(hdc, 0, 0, bw as i32, bh as i32, 16, 16);
            SelectObject(hdc, old_brush as _);
            DeleteObject(brush as _);
        }
    } else {
        unsafe {
            let old_brush = SelectObject(hdc, GetStockObject(WHITE_BRUSH) as _);
            RoundRect(hdc, 0, 0, bw as i32, bh as i32, 16, 16);
            SelectObject(hdc, old_brush as _);
        }
    }
    // 深色文字（透明背景）
    unsafe {
        SetBkMode(hdc, TRANSPARENT as i32);
        SetTextColor(hdc, 0x001E_1E1E);
        let mut rc = RECT {
            left: pad_x,
            top: 0,
            right: bw as i32 - pad_x,
            bottom: bh as i32,
        };
        DrawTextW(
            hdc,
            text_w.as_ptr(),
            text_w.len() as i32,
            &mut rc,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
        );
        // 读回像素
        let mut buf = vec![0u8; bw * bh * 4];
        std::ptr::copy_nonoverlapping(bits as *const u8, buf.as_mut_ptr(), buf.len());
        SelectObject(hdc, old_bmp as _);
        SelectObject(hdc, old_font as _);
        DeleteObject(bmp as _);
        DeleteObject(font as _);
        DeleteDC(hdc);
        return Some(BubbleBitmap { buf, w: bw, h: bh });
    }
}

/// 把气泡位图合成到窗口像素缓冲顶部中央（premultiplied alpha 混合）。
fn composite_bubble(dst: &mut [u8], dw: usize, dh: usize, b: &BubbleBitmap) {
    if b.w == 0 || b.h == 0 || b.w >= dw {
        return;
    }
    let bx = ((dw - b.w) / 2).max(0);
    let by = 4usize;
    for y in 0..b.h {
        let dy = by + y;
        if dy >= dh {
            break;
        }
        for x in 0..b.w {
            let dx = bx + x;
            if dx >= dw {
                break;
            }
            let si = (y * b.w + x) * 4;
            let sa = b.buf[si + 3];
            if sa == 0 {
                continue;
            }
            let di = (dy * dw + dx) * 4;
            let inv = 255 - sa;
            dst[di] = ((b.buf[si] as u32 * sa as u32 + dst[di] as u32 * inv as u32) / 255) as u8;
            dst[di + 1] =
                ((b.buf[si + 1] as u32 * sa as u32 + dst[di + 1] as u32 * inv as u32) / 255) as u8;
            dst[di + 2] =
                ((b.buf[si + 2] as u32 * sa as u32 + dst[di + 2] as u32 * inv as u32) / 255) as u8;
            dst[di + 3] = 255;
        }
    }
}

/// 全局光标位置（物理像素）。
pub fn cursor_pos() -> (i32, i32) {
    let mut p: POINT = unsafe { std::mem::zeroed() };
    let ok = unsafe { GetCursorPos(&mut p) };
    if ok != 0 {
        (p.x, p.y)
    } else {
        (i32::MIN, i32::MIN)
    }
}

/// 请求退出消息循环。
pub fn post_quit() {
    unsafe { PostQuitMessage(0) };
}

/// 用系统默认浏览器打开 URL（打开控制台）。
pub fn open_url(url: &str) {
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let u: Vec<u16> = url.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            windows_sys::core::w!("open"),
            u.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        );
    }
}

/// 消息循环。
pub fn message_loop() -> i32 {
    let mut msg: MSG = unsafe { std::mem::zeroed() };
    unsafe {
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    0
}

/// 把菜单数据渲染成 Win32 弹出菜单并阻塞显示，返回选中的命令 ID（0 = 未选）。
pub fn show_menu_blocking(parent: HWND, items: &[MenuEntry], x: i32, y: i32) -> usize {
    let menu = unsafe { CreatePopupMenu() };
    append_menu_items(menu, items);
    let cmd = unsafe {
        TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON,
            x,
            y,
            0,
            parent,
            std::ptr::null(),
        )
    };
    unsafe { DestroyMenu(menu) };
    cmd as usize
}

fn append_menu_items(menu: HMENU, items: &[MenuEntry]) {
    for it in items {
        if it.is_separator() {
            unsafe { AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null()) };
            continue;
        }
        if let Some(children) = &it.children {
            let sub = unsafe { CreatePopupMenu() };
            append_menu_items(sub, children);
            unsafe { AppendMenuW(menu, MF_STRING | MF_POPUP, sub as usize, wide(&it.text)) };
        } else {
            let flags = MF_STRING | if it.checked { MF_CHECKED } else { 0 };
            unsafe { AppendMenuW(menu, flags, it.id as usize, wide(&it.text)) };
        }
    }
}

fn wide(s: &str) -> *const u16 {
    let mut v: Vec<u16> = s.encode_utf16().collect();
    v.push(0);
    let ptr = v.as_ptr();
    std::mem::forget(v);
    ptr
}
