//! macOS 后端：AppKit 原生实现。
//!
//! - NSWindow 透明无边框 + 自定义 NSView 逐帧 drawRect 渲染（CGBitmapContext → CGImage）
//! - 按像素 alpha 命中测试 + 窗口级 ignoresMouseEvents 动态切换（透明像素点击落到下层窗口）
//! - NSStatusItem 托盘 + NSMenu（target-action 按 tag 分发命令）
//! - NSTimer 帧驱动（10ms）
//! - 坐标体系：与 win32 一致采用"左上原点"（点）。NSEvent::mouseLocation /
//!   NSScreen.visibleFrame 均为左下原点，进入时换算。
//!
//! 注意：本模块仅能在 macOS 上编译/运行；在 Windows 上仅做 cargo check 类型验证。
#![allow(non_snake_case, dead_code)]

use std::ffi::c_void;
use std::ptr::NonNull;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{define_class, msg_send, sel, MainThreadOnly};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSColor, NSGraphicsContext,
    NSImage, NSMenu, NSMenuItem, NSScreen, NSStatusBar, NSStatusItem, NSView, NSWindow,
    NSWindowCollectionBehavior, NSWindowStyleMask, NSFloatingWindowLevel, NSNormalWindowLevel,
};
use objc2_core_graphics::{CGColorSpace, CGContext, CGImage};
use objc2_foundation::{
    ns_string, MainThreadMarker, NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString,
    NSTimer,
};

use crate::app::App;
use crate::menu::MenuEntry;

// ---------------- raw FFI：CoreGraphics 旧 C API（objc2-core-graphics 未生成） ----------------

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C-unwind" {
    fn CGBitmapContextCreate(
        data: *mut c_void,
        width: usize,
        height: usize,
        bits_per_component: usize,
        bytes_per_row: usize,
        space: *const CGColorSpace,
        bitmap_info: u32,
    ) -> *mut CGContext;
    fn CGColorSpaceCreateDeviceRGB() -> *mut CGColorSpace;
    fn CGColorSpaceRelease(space: *mut CGColorSpace);
    fn CGBitmapContextCreateImage(ctx: *mut CGContext) -> *mut CGImage;
    fn CGImageRelease(image: *mut CGImage);
    fn CGImageCreateCopy(image: *mut CGImage) -> *mut CGImage;
    fn CGContextRelease(ctx: *mut CGContext);
    fn CGContextDrawImage(ctx: *mut CGContext, rect: NSRect, image: *mut CGImage);
}

/// kCGBitmapByteOrder32Little | kCGImageAlphaPremultipliedFirst → 内存字节序 BGRA premultiplied（与 win32 渲染缓冲一致）。
/// 注意：kCGBitmapByteOrder32Little = (2 << 12)，写成 (1 << 12) 会变成 16Little，
/// 与 8 bit/component 组合非法，CGBitmapContextCreate 返回 NULL（真机渲染全透明）。
const BITMAP_INFO: u32 = (2 << 12) | 2;

/// 托盘菜单命令区间起点（>= 该值走 handle_tray_command，否则走 handle_command）。
const TRAY_CMD_BASE: isize = 1001;

// ---------------- 全局单线程指针 ----------------

static mut APP: *mut App = std::ptr::null_mut();
static mut STATUS_ITEM: *mut NSStatusItem = std::ptr::null_mut();
static mut MENU_HANDLER: *mut PetMenuHandler = std::ptr::null_mut();

// ---------------- 自定义视图 ----------------

define_class!(
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "DeskpetPetView"]
    struct PetView;

    unsafe impl NSObjectProtocol for PetView {}

    impl PetView {
        /// 穿透不在这里做：hitTest 返回 nil 只会吞掉点击（不会穿透到下层窗口）。
        /// 改用窗口级 ignoresMouseEvents 动态切换（见 PetWindow::update_mouse_passthrough），
        /// 该机制在窗口服务器层生效，透明区域点击会真正落到下层窗口。

        /// 无边框窗口默认可被拖拽移动窗口，与桌宠自身拖拽冲突，禁用。
        #[unsafe(method(mouseDownCanMoveWindow))]
        fn mouse_down_can_move_window(&self) -> bool {
            false
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &objc2_app_kit::NSEvent) {
            let (x, y) = self.event_client_point(event);
            unsafe {
                if !APP.is_null() {
                    (&mut *APP).on_pet_press(x, y);
                }
            }
        }

        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged(&self, _event: &objc2_app_kit::NSEvent) {
            unsafe {
                if !APP.is_null() {
                    (&mut *APP).on_pet_drag();
                }
            }
        }

        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, _event: &objc2_app_kit::NSEvent) {
            unsafe {
                if !APP.is_null() {
                    (&mut *APP).on_pet_release();
                }
            }
        }

        // 右键菜单已迁移到状态栏（托盘）：桌宠身上右键不再弹菜单（与 win32 行为一致）。

        /// drawRect：把 CGBitmapContext 的当前图像画到视图。
        /// 视图未翻转（isFlipped=NO）：CGContextDrawImage 对"首行为顶行"的 CGImage 直接正立绘制。
        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty: NSRect) {
            unsafe {
                if APP.is_null() {
                    return;
                }
                let app = &mut *APP;
                let Some(pet) = app.pet.as_mut() else { return };
                let img = pet.win.make_image();
                if img.is_null() {
                    return;
                }
                let Some(ctx) = NSGraphicsContext::currentContext() else {
                    CGImageRelease(img);
                    return;
                };
                let ctx = ctx.CGContext();
                let bounds = self.bounds();
                CGContextDrawImage(
                    &*ctx as *const CGContext as *mut CGContext,
                    NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(bounds.size.width, bounds.size.height)),
                    img,
                );
                // make_image 返回 +1 引用（CGBitmapContextCreateImage），必须配对释放，
                // 否则每帧泄漏一个 CGImage → 内存线性增长
                CGImageRelease(img);
                // 说话气泡（say API）
                if let Some(bubble) = &pet.bubble {
                    draw_bubble(&*ctx as *const CGContext as *mut CGContext, bounds, bubble);
                }
            }
        }
    }
);

impl PetView {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        // set_ivars 返回 PartialInit（super 调用 init 家族方法所需）
        let this = Self::alloc(mtm).set_ivars(());
        unsafe {
            msg_send![
                super(this),
                initWithFrame: NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(1.0, 1.0))
            ]
        }
    }

    /// 事件窗口坐标 → 视图客户区坐标（左上原点，点）。
    fn event_client_point(&self, event: &objc2_app_kit::NSEvent) -> (i32, i32) {
        let p = event.locationInWindow();
        let vp = self.convertPoint_fromView(p, None);
        let h = self.bounds().size.height;
        (vp.x as i32, (h - vp.y) as i32)
    }
}

/// 说话气泡：圆角矩形 + 文本（say API，平台层绘制，不依赖素材）。
/// 视图未翻转（左下原点），气泡放在视图顶部中央。
fn draw_bubble(ctx: *mut CGContext, bounds: NSRect, bubble: &crate::pet::Bubble) {
    use objc2_app_kit::{
        NSStringDrawing, NSStringDrawingOptions, NSStringNSExtendedStringDrawing, NSBezierPath,
    };
    let text = NSString::from_str(&bubble.text);
    // 单行尺寸（默认属性：系统字体 12pt 黑色文字）
    let single = unsafe { text.sizeWithAttributes(None) };
    let pad_x = 14.0;
    let pad_y = 7.0;
    let max_w = (bounds.size.width - 12.0).max(48.0);
    let bw = (single.width + pad_x * 2.0).clamp(48.0, max_w);
    // 按实际宽度重新测量（自动换行后的高度），避免长文本被裁剪
    let text_w = (bw - pad_x * 2.0).max(1.0);
    let wrapped = unsafe {
        text.boundingRectWithSize_options_attributes_context(
            NSSize::new(text_w, f64::INFINITY),
            NSStringDrawingOptions::UsesLineFragmentOrigin,
            None,
            None,
        )
    };
    let bh = (wrapped.size.height + pad_y * 2.0 + 2.0).max(26.0);
    let bx = (bounds.size.width - bw) / 2.0;
    let by = (bounds.size.height - bh - 6.0).max(2.0); // 顶部留 6pt
    let rect = NSRect::new(NSPoint::new(bx, by), NSSize::new(bw, bh));

    let path = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(rect, 8.0, 8.0);
    unsafe {
        // 背景白底 + 灰色描边 + 深色文字
        NSColor::whiteColor().setFill();
        path.fill();
        NSColor::grayColor().setStroke();
        path.setLineWidth(1.0);
        path.stroke();
        let tr = NSRect::new(
            NSPoint::new(bx + pad_x, by + pad_y),
            NSSize::new((bw - pad_x * 2.0).max(1.0), (bh - pad_y * 2.0).max(1.0)),
        );
        text.drawInRect_withAttributes(tr, None);
    }
    let _ = ctx; // 文本/图形经 AppKit 绘制到当前上下文，无需直接使用 ctx
}

// ---------------- 菜单处理器（status menu 的 target） ----------------

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    struct PetMenuHandler;

    unsafe impl NSObjectProtocol for PetMenuHandler {}

    impl PetMenuHandler {
        #[unsafe(method(handleCommand:))]
        fn handle_command(&self, sender: &AnyObject) {
            unsafe {
                if let Some(item) = sender.downcast_ref::<NSMenuItem>() {
                    let tag = item.tag();
                    if tag > 0 && !APP.is_null() {
                        let app = &mut *APP;
                        if tag >= TRAY_CMD_BASE {
                            app.handle_tray_command(tag as usize);
                        } else {
                            app.handle_command(tag as usize);
                        }
                    }
                }
            }
        }
    }
);

impl PetMenuHandler {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        unsafe { msg_send![super(this), init] }
    }
}

// ---------------- 菜单数据 → NSMenu ----------------

fn build_ns_menu(items: &[MenuEntry]) -> Retained<NSMenu> {
    let mtm = MainThreadMarker::new().expect("main thread");
    let menu = NSMenu::new(mtm);
    menu.setAutoenablesItems(false);
    for it in items {
        if it.is_separator() {
            menu.addItem(&NSMenuItem::separatorItem(mtm));
        } else if let Some(children) = &it.children {
            let sub = build_ns_menu(children);
            let item = unsafe {
                NSMenuItem::initWithTitle_action_keyEquivalent(
                    NSMenuItem::alloc(mtm),
                    &NSString::from_str(&it.text),
                    None,
                    ns_string!(""),
                )
            };
            item.setSubmenu(Some(&sub));
            menu.addItem(&item);
        } else {
            let item = unsafe {
                NSMenuItem::initWithTitle_action_keyEquivalent(
                    NSMenuItem::alloc(mtm),
                    &NSString::from_str(&it.text),
                    Some(sel!(handleCommand:)),
                    ns_string!(""),
                )
            };
            item.setTag(it.id as isize);
            // NSControlStateValue = NSInteger：1 = On，0 = Off
            item.setState(if it.checked { 1 } else { 0 });
            unsafe {
                if !MENU_HANDLER.is_null() {
                    let handler = MENU_HANDLER as *const PetMenuHandler as *const AnyObject;
                    item.setTarget(Some(&*handler));
                }
            }
            menu.addItem(&item);
        }
    }
    menu
}

// ---------------- 窗口 ----------------

pub struct PetWindow {
    window: Retained<NSWindow>,
    view: Retained<PetView>,
    pix_w: usize,
    pix_h: usize,
    buffer: Vec<u8>,
    alpha: Vec<u8>,
    ctx: *mut CGContext,
    color_space: *mut CGColorSpace,
    /// 当前是否忽略鼠标事件（穿透）。
    ignores_mouse: bool,
}

impl PetWindow {
    /// 创建透明置顶窗口（点尺寸 1×1，resize 时定形）。
    pub fn create(on_top: bool) -> Option<PetWindow> {
        let mtm = MainThreadMarker::new()?;
        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(mtm),
                NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(1.0, 1.0)),
                NSWindowStyleMask::Borderless,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        // 非窗口控制器创建的窗口必须禁止关闭时自动释放
        unsafe { window.setReleasedWhenClosed(false) };
        window.setOpaque(false);
        window.setBackgroundColor(Some(&NSColor::clearColor()));
        window.setHasShadow(false);
        window.setLevel(if on_top {
            NSFloatingWindowLevel
        } else {
            NSNormalWindowLevel
        });
        window.setCollectionBehavior(
            NSWindowCollectionBehavior::CanJoinAllSpaces
                | NSWindowCollectionBehavior::Stationary
                | NSWindowCollectionBehavior::FullScreenAuxiliary,
        );
        // 初始全穿透；每 tick 按光标位置动态切换（update_mouse_passthrough）
        window.setIgnoresMouseEvents(true);
        let view = PetView::new(mtm);
        window.setContentView(Some(&view));
        let mut w = PetWindow {
            window,
            view,
            pix_w: 1,
            pix_h: 1,
            buffer: vec![0u8; 4],
            alpha: vec![0u8; 1],
            ctx: std::ptr::null_mut(),
            color_space: std::ptr::null_mut(),
            ignores_mouse: true,
        };
        w.resize(1, 1);
        Some(w)
    }

    /// 每 tick 调用：光标位于不透明像素上时关闭穿透（可交互），否则开启（点击落到下层窗口）。
    /// macOS 无逐像素命中测试消息（对比 win32 的 WM_NCHITTEST），此为窗口服务器层可靠方案。
    pub fn update_mouse_passthrough(&mut self) {
        let (gx, gy) = crate::platform::cursor_pos();
        let (l, t, r, b) = self.get_rect();
        let inside = gx >= l && gx < r && gy >= t && gy < b;
        let opaque = inside && self.hit_test_alpha(gx - l, gy - t);
        let should_ignore = !opaque;
        if should_ignore != self.ignores_mouse {
            self.ignores_mouse = should_ignore;
            self.window.setIgnoresMouseEvents(should_ignore);
        }
    }

    /// 重建位图缓冲（w/h 为设备像素；窗口点尺寸 = 像素 / backingScaleFactor）。
    /// 尺寸不变时跳过，避免每帧重建 CGBitmapContext。
    pub fn resize(&mut self, w: i32, h: i32) {
        if w < 1 || h < 1 {
            return;
        }
        if w as usize == self.pix_w && h as usize == self.pix_h {
            return;
        }
        let w = w as usize;
        let h = h as usize;
        let backing = self.window.backingScaleFactor();
        let w_pt = w as f64 / backing;
        let h_pt = h as f64 / backing;
        self.window.setContentSize(NSSize::new(w_pt, h_pt));

        // 先建新缓冲（堆数据），再以新缓冲为数据源重建 CGBitmapContext，
        // 最后替换字段（Vec 移动不改变堆指针，context 引用始终有效）。
        let new_buf = vec![0u8; w * h * 4];
        let new_alpha = vec![0u8; w * h];
        if !self.ctx.is_null() {
            unsafe { CGContextRelease(self.ctx) };
        }
        if !self.color_space.is_null() {
            unsafe { CGColorSpaceRelease(self.color_space) };
        }
        let cs = unsafe { CGColorSpaceCreateDeviceRGB() };
        let ctx = unsafe {
            CGBitmapContextCreate(
                new_buf.as_ptr() as *mut c_void,
                w,
                h,
                8,
                w * 4,
                cs,
                BITMAP_INFO,
            )
        };
        self.ctx = ctx;
        self.color_space = cs;
        self.buffer = new_buf;
        self.alpha = new_alpha;
        self.pix_w = w;
        self.pix_h = h;
    }

    pub fn show(&self) {
        self.window.orderFrontRegardless();
    }

    pub fn hide(&self) {
        self.window.orderOut(None);
    }

    /// 左上原点（点）。
    pub fn move_to(&self, x: i32, y: i32) {
        let frame = self.window.frame();
        let sh = screen_height_points();
        let origin = NSPoint::new(x as f64, sh - y as f64 - frame.size.height);
        self.window.setFrameOrigin(origin);
    }

    pub fn set_topmost(&self, on: bool) {
        self.window.setLevel(if on {
            NSFloatingWindowLevel
        } else {
            NSNormalWindowLevel
        });
    }

    /// 左上原点（点）：(left, top, right, bottom)。
    pub fn get_rect(&self) -> (i32, i32, i32, i32) {
        let frame = self.window.frame();
        let sh = screen_height_points();
        let l = frame.origin.x;
        let t = sh - frame.origin.y - frame.size.height;
        let r = frame.origin.x + frame.size.width;
        let b = sh - frame.origin.y;
        (l as i32, t as i32, r as i32, b as i32)
    }

    /// 渲染：把 src（sw×sh BGRA）缩放到窗口并请求重绘。
    pub fn present(&mut self, src: &[u8], sw: usize, sh: usize, mirror: bool) {
        if src.len() < sw * sh * 4 || self.pix_w == 0 || self.pix_h == 0 {
            return;
        }
        crate::gfx::scale_bgra(
            src,
            sw,
            sh,
            &mut self.buffer,
            self.pix_w,
            self.pix_h,
            self.pix_w * 4,
            mirror,
        );
        for y in 0..self.pix_h {
            let row = y * self.pix_w;
            for x in 0..self.pix_w {
                self.alpha[row + x] = self.buffer[(row + x) * 4 + 3];
            }
        }
        self.view.setNeedsDisplay(true);
    }

    /// 命中测试（参数为客户区坐标，点；内部换算到设备像素）。
    pub fn hit_test_alpha(&self, x: i32, y: i32) -> bool {
        if x < 0 || y < 0 {
            return false;
        }
        let backing = self.window.backingScaleFactor();
        let px = (x as f64 * backing) as usize;
        let py = (y as f64 * backing) as usize;
        if px >= self.pix_w || py >= self.pix_h {
            return false;
        }
        self.alpha[py * self.pix_w + px] >= 128
    }

    /// 当前帧 CGImage（引用存活中的 CGBitmapContext 数据；调用方须在本窗口存活期间使用）。
    pub fn make_image(&self) -> *mut CGImage {
        if self.ctx.is_null() {
            return std::ptr::null_mut();
        }
        unsafe { CGBitmapContextCreateImage(self.ctx) }
    }

    /// macOS 由全局 NSTimer 驱动，此方法为空实现（与 win32 接口对齐）。
    pub fn start_frame_timer(&self, _interval_ms: u32) {}
}

impl Drop for PetWindow {
    fn drop(&mut self) {
        if !self.ctx.is_null() {
            unsafe { CGContextRelease(self.ctx) };
        }
        if !self.color_space.is_null() {
            unsafe { CGColorSpaceRelease(self.color_space) };
        }
    }
}

// ---------------- 平台函数 ----------------

/// 主屏高度（点）。
fn screen_height_points() -> f64 {
    let mtm = match MainThreadMarker::new() {
        Some(m) => m,
        None => return 0.0,
    };
    NSScreen::mainScreen(mtm)
        .map(|s| s.frame().size.height)
        .unwrap_or(0.0)
}

/// 全局光标位置（点，左上原点）。
pub fn cursor_pos() -> (i32, i32) {
    let p = objc2_app_kit::NSEvent::mouseLocation();
    let sh = screen_height_points();
    (p.x as i32, (sh - p.y) as i32)
}

/// 请求退出。
pub fn post_quit() {
    let Some(mtm) = MainThreadMarker::new() else { return };
    NSApplication::sharedApplication(mtm).terminate(None);
}

/// 用系统默认浏览器打开 URL（打开控制台）。
pub fn open_url(url: &str) {
    use objc2_app_kit::NSWorkspace;
    use objc2_foundation::NSURL;
    let s = NSString::from_str(url);
    let Some(nsurl) = NSURL::URLWithString(&s) else {
        log_warn!("URL 无效: {}", url);
        return;
    };
    let ws = NSWorkspace::sharedWorkspace();
    if !ws.openURL(&nsurl) {
        log_warn!("打开 URL 失败: {}", url);
    }
}

// ---------------- 状态栏 / 定时器 / 主循环 ----------------

/// 从当前渲染帧生成状态栏图标（CGImage 独立副本 → NSImage）。
fn make_status_image(app: &mut App) -> Option<Retained<NSImage>> {
    let pet = match app.pet.as_mut() {
        Some(p) => p,
        // 无素材（未导入）时用默认圆形图标，保证状态栏可点
        None => return default_status_image(),
    };
    let img = pet.win.make_image();
    if img.is_null() {
        return None;
    }
    let copy = unsafe { CGImageCreateCopy(img) };
    if copy.is_null() {
        return None;
    }
    let mtm = MainThreadMarker::new()?;
    // NSImage 持有传入的 CGImage（会 retain）；释放我们自己的 +1 引用
    let nsimg = unsafe {
        let nsimg = NSImage::initWithCGImage_size(mtm.alloc(), &*copy, NSSize::new(32.0, 18.0));
        CGImageRelease(copy);
        nsimg
    };
    Some(nsimg)
}

/// 无素材时的默认状态栏图标（32×18 椭圆，BGRA premultiplied）。
fn default_status_image() -> Option<Retained<NSImage>> {
    let mtm = MainThreadMarker::new()?;
    let (w, h) = (32usize, 18usize);
    let mut buf = vec![0u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let dx = x as f64 - w as f64 / 2.0;
            let dy = y as f64 - h as f64 / 2.0;
            if dx * dx / (13.5 * 13.5) + dy * dy / (7.0 * 7.0) <= 1.0 {
                let i = (y * w + x) * 4;
                buf[i] = 0x7f;
                buf[i + 1] = 0x9c;
                buf[i + 2] = 0xff;
                buf[i + 3] = 255;
            }
        }
    }
    unsafe {
        let cs = CGColorSpaceCreateDeviceRGB();
        if cs.is_null() {
            return None;
        }
        let ctx = CGBitmapContextCreate(buf.as_mut_ptr() as *mut c_void, w, h, 8, w * 4, cs, BITMAP_INFO);
        CGColorSpaceRelease(cs);
        if ctx.is_null() {
            return None;
        }
        let img = CGBitmapContextCreateImage(ctx);
        CGContextRelease(ctx);
        if img.is_null() {
            return None;
        }
        // NSImage 会 retain img；释放我们自己的 +1 引用
        let nsimg = NSImage::initWithCGImage_size(mtm.alloc(), &*img, NSSize::new(32.0, 18.0));
        CGImageRelease(img);
        Some(nsimg)
    }
}

fn setup_status_item(mtm: MainThreadMarker, app: &mut App) {
    let bar = NSStatusBar::systemStatusBar();
    let item = bar.statusItemWithLength(-1.0); // NSStatusItemVariableLength
    if let Some(btn) = item.button(mtm) {
        if let Some(img) = make_status_image(app) {
            btn.setImage(Some(&img));
        }
        btn.setToolTip(Some(ns_string!("deskpet 桌宠")));
    }
    // 完整菜单：桌宠菜单（角落/置顶/不移动/自启/大小）+ 打开控制台/显示隐藏/退出（与 win32 托盘右键一致）
    let mut items = app
        .pet
        .as_ref()
        .map(|p| p.context_menu_items())
        .unwrap_or_default();
    items.push(MenuEntry::separator());
    items.push(MenuEntry::item(crate::tray::TRAY_CONSOLE, "打开控制台"));
    items.push(MenuEntry::item(crate::tray::TRAY_TOGGLE_VISIBLE, "显示/隐藏"));
    items.push(MenuEntry::item(crate::tray::TRAY_QUIT, "退出"));
    let menu = build_ns_menu(&items);
    item.setMenu(Some(&menu));
    // 应用生命周期内不释放
    unsafe { STATUS_ITEM = Retained::into_raw(item) };
}

fn setup_timer(app_ptr: *mut App) {
    let ptr = app_ptr as usize;
    let block = RcBlock::new(move |_timer: NonNull<NSTimer>| {
        if ptr != 0 {
            unsafe {
                let app = &mut *(ptr as *mut App);
                app.on_pet_tick();
                // 光标位置 → 窗口穿透状态（透明像素点击落到下层窗口）
                if let Some(pet) = app.pet.as_mut() {
                    pet.win.update_mouse_passthrough();
                }
            }
        }
    });
    unsafe {
        // 已调度进当前 run loop，retained 由 run loop 持有
        let _t = NSTimer::scheduledTimerWithTimeInterval_repeats_block(0.01, true, &*block);
    }
}

/// AppKit 主循环（阻塞直到退出）。
pub fn run(app: &mut App) {
    let mtm = MainThreadMarker::new().expect("必须在主线程运行");
    unsafe { APP = app as *mut App };

    // 菜单处理器 + 状态栏 + 帧定时器
    let handler = PetMenuHandler::new(mtm);
    unsafe { MENU_HANDLER = Retained::into_raw(handler) };
    setup_status_item(mtm, app);
    setup_timer(unsafe { APP });

    let nsapp = NSApplication::sharedApplication(mtm);
    nsapp.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    nsapp.run();
    // run 返回即退出流程（quit_all 已保存位置）
}
