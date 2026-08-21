//! 单只桌宠：窗口 + 动画状态机 + 交互 + 渲染（平台无关核心）。
//! 平台差异（窗口创建/事件来源/菜单渲染/光标）由 win32.rs / macos.rs 桥接层处理。
#![allow(non_snake_case, dead_code)]

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use crate::assets::RoleAssets;
use crate::clip::{ClipDecoder, H, W};
use crate::config::PetConfig;
use crate::menu::MenuEntry;
use crate::platform::{cursor_pos, PetWindow};
use crate::state::{self, Category};

// 菜单 ID
pub const MID_CORNER: usize = 190;
pub const MID_ONTOP: usize = 191;
pub const MID_NOMOVE: usize = 192;
pub const MID_AUTOSTART: usize = 193;
pub const MID_SCALE_BASE: usize = 200;

/// 说话气泡（平台层绘制，不依赖素材）。
pub struct Bubble {
    pub text: String,
    pub until: Instant,
}

/// 外部指令（HTTP API 下发），高优先级插入动画链状态机。
pub enum PetCommand {
    /// 播放指定动作（已解析的动作名）。
    Play(String),
    /// 移动到工作区归一化位置（0..1）。
    MoveTo { x: f64, y: f64 },
    /// 说话气泡。
    Say { text: String, duration_ms: Option<u64> },
}

/// 行为引擎参数（来自 PetConfig，阶段 3 配置化；见 docs/需求规格.md §5.2）。
#[derive(Clone, Debug)]
pub struct Behavior {
    /// 待机分支概率（0..1，须 < turn_ratio）。
    pub idle_ratio: f64,
    /// 转向分支累积概率。
    pub turn_ratio: f64,
    /// 闲时动作分支累积概率（剩余为移动）。
    pub act_ratio: f64,
    /// 普通动画结束后停顿毫秒数（0 = 立即继续）。
    pub act_interval_ms: u64,
    /// 自主移动最小距离（像素）。
    pub move_min_px: f64,
    /// 自主移动最大距离（像素）。
    pub move_max_px: f64,
    /// 移动边界留白（像素）。
    pub move_margin_px: f64,
    /// 缩放档位（大小菜单/设置页）。
    pub scale_steps: Vec<f64>,
}

impl From<&PetConfig> for Behavior {
    fn from(pc: &PetConfig) -> Behavior {
        Behavior {
            idle_ratio: pc.idle_ratio,
            turn_ratio: pc.turn_ratio,
            act_ratio: pc.act_ratio,
            act_interval_ms: pc.act_interval_ms,
            move_min_px: pc.move_min_px,
            move_max_px: pc.move_max_px,
            move_margin_px: pc.move_margin_px,
            scale_steps: pc.scale_steps.clone(),
        }
    }
}

#[derive(Clone)]
struct MovePlan {
    start_x: i32,
    target_x: i32,
    y: i32,
    duration_ms: u64,
}

pub struct Pet {
    pub win: PetWindow,
    pub cats: Category,
    pub clips: HashMap<String, ClipDecoder>,
    pub cur_anim: String,
    pub facing_right: bool,
    pub scale: f64,
    pub no_move: bool,
    pub win_topmost: bool,
    pub visible: bool,
    pub render_buf: Vec<u8>,
    /// 共享 VP9 解码器（全部动画共用一组；一次只播一个动画，首帧关键帧重置
    /// 参考帧状态。避免每动画独立解码器导致 ~6MB×动画数的帧缓冲池累积）
    color_dec: crate::vpx::Decoder,
    alpha_dec: Option<crate::vpx::Decoder>,
    /// 共享合成缓冲（全部动画共用，避免 51×921KB 独立缓冲）
    comp_buf: Vec<u8>,
    pub frame_accum_ms: u64,
    pub anim_ended_fired: bool,
    /// 说话气泡（None = 无）。
    pub bubble: Option<Bubble>,
    /// 外部指令队列（HTTP API 下发，tick 内高优先级执行）。
    cmd_queue: VecDeque<PetCommand>,

    press_global: Option<(i32, i32)>,
    grab_offset: Option<(i32, i32)>,
    dragging: bool,
    just_dragged: bool,

    move_plan: Option<MovePlan>,
    move_accum_ms: u64,

    /// 行为引擎参数（程序级，可热更新）。
    pub behavior: Behavior,
    /// 动作间隔冷却（普通动画结束后等待，None = 无需等待）。
    cooldown_until: Option<Instant>,

    last_tick: Instant,
}

/// 从素材集构建解码器 + 动态分类（分类来自管理端动作配置）。
fn build_from_role(
    role: &RoleAssets,
    actions: &HashMap<String, (String, f64, bool)>,
) -> (HashMap<String, ClipDecoder>, Category) {
    let mut clips: HashMap<String, ClipDecoder> = HashMap::new();
    for (name, wm) in &role.videos {
        if let Some(dec) = ClipDecoder::new(wm.clone()) {
            clips.insert(name.clone(), dec);
        }
    }
    let cats = state::build_categories_from_actions(&role.names, actions);
    log_info!(
        "动画分类: idle={:?} turn={:?} moves={} clicks={} drag={:?} acts={}",
        cats.idle,
        cats.turn,
        cats.moves.len(),
        cats.clicks.len(),
        cats.drag,
        cats.acts.len()
    );
    (clips, cats)
}

impl Pet {
    pub fn new(
        win: PetWindow,
        role: &RoleAssets,
        pc: &PetConfig,
        actions: &HashMap<String, (String, f64, bool)>,
    ) -> Pet {
        let (clips, cats) = build_from_role(role, actions);
        let mut pet = Pet {
            win,
            cats,
            clips,
            cur_anim: String::new(),
            facing_right: pc.facing_right,
            scale: pc.scale,
            no_move: pc.no_move,
            win_topmost: pc.always_on_top,
            visible: true,
            render_buf: vec![0u8; W * (H + 30) * 4],
            color_dec: crate::vpx::Decoder::new(4).expect("libvpx 主色解码器初始化失败"),
            alpha_dec: crate::vpx::Decoder::new(2),
            comp_buf: Vec::new(),
            frame_accum_ms: 0,
            anim_ended_fired: false,
            bubble: None,
            cmd_queue: VecDeque::new(),
            press_global: None,
            grab_offset: None,
            dragging: false,
            just_dragged: false,
            move_plan: None,
            move_accum_ms: 0,
            behavior: Behavior::from(pc),
            cooldown_until: None,
            last_tick: Instant::now(),
        };
        let (w, h) = pet.window_size();
        pet.win.resize(w, h);
        pet.win.set_topmost(pc.always_on_top);
        let idle = pet.cats.idle.clone().unwrap_or_default();
        pet.switch_anim(&idle);
        pet.win.show();
        pet.win.start_frame_timer(10);
        pet
    }

    pub fn window_size(&self) -> (i32, i32) {
        let w = (state::CANVAS_W * self.scale).round() as i32;
        let h = ((state::CANVAS_H + state::PAD) * self.scale).round() as i32;
        (w.max(1), h.max(1))
    }

    /// 热替换素材集（导入后 / 角色切换）：重建 clips/cats，重播待机动画，保留窗口与位置。
    pub fn swap_role(&mut self, role: &RoleAssets, actions: &HashMap<String, (String, f64, bool)>) {
        let (clips, cats) = build_from_role(role, actions);
        self.clips = clips;
        self.cats = cats;
        self.cancel_move();
        self.move_plan = None;
        self.cur_anim = String::new();
        self.anim_ended_fired = false;
        let idle = self.cats.idle.clone().unwrap_or_default();
        self.switch_anim(&idle);
        log_info!("素材已热替换（{} 段动画）", self.clips.len());
    }

    // ---------------- 外部指令（HTTP API，docs/需求规格.md §7） ----------------

    /// 入队外部指令（下个 tick 高优先级执行）。
    pub fn enqueue(&mut self, cmd: PetCommand) {
        self.cmd_queue.push_back(cmd);
    }

    fn exec_command(&mut self, cmd: PetCommand) {
        match cmd {
            PetCommand::Play(name) => {
                if let Some(resolved) = self.resolve_action(&name) {
                    log_info!("指令 play: {} -> {}", name, resolved);
                    self.switch_anim(&resolved);
                } else {
                    log_warn!("指令 play: 未找到动作 {}", name);
                }
            }
            PetCommand::MoveTo { x, y } => self.move_to_normalized(x, y),
            PetCommand::Say { text, duration_ms } => {
                let ms = duration_ms.unwrap_or(4000).max(500);
                self.bubble = Some(Bubble {
                    text,
                    until: Instant::now() + Duration::from_millis(ms),
                });
            }
        }
    }

    /// 动作名解析：精确名 → 语义名（idle/turn/move/act/click/drag）→ 子串最近匹配。
    pub fn resolve_action(&self, query: &str) -> Option<String> {
        let q = query.trim();
        if q.is_empty() {
            return None;
        }
        if self.clips.contains_key(q) {
            return Some(q.to_string());
        }
        let sem = match q.to_ascii_lowercase().as_str() {
            "idle" => self.cats.idle.clone(),
            "turn" => self.cats.turns.first().cloned(),
            "move" | "moves" => self.cats.moves.first().cloned(),
            "act" | "acts" => self.cats.acts.first().cloned(),
            "click" | "clicks" => self.cats.clicks.first().cloned(),
            "drag" => self.cats.drag.clone(),
            _ => None,
        };
        if let Some(n) = sem {
            return Some(n);
        }
        self.clips
            .keys()
            .find(|n| n.contains(q) || q.contains(n.as_str()))
            .cloned()
    }

    /// 移动到工作区归一化位置（0..1，立即生效，不播放移动动画）。
    pub fn move_to_normalized(&mut self, x: f64, y: f64) {
        let (lx, ly, rx, ry) = self.avail_rect();
        let (wx, wh) = self.window_size();
        let tx = (lx as f64 + x.clamp(0.0, 1.0) * (rx - lx) as f64) as i32 - wx / 2;
        let ty = (ly as f64 + y.clamp(0.0, 1.0) * (ry - ly) as f64) as i32 - wh / 2;
        let tx = tx.clamp(lx, (rx - wx).max(lx));
        let ty = ty.clamp(ly, (ry - wh).max(ly));
        self.cancel_move();
        self.win.move_to(tx, ty);
        log_info!("指令 move: ({:.2}, {:.2}) -> ({}, {})", x, y, tx, ty);
    }

    pub fn switch_anim(&mut self, name: &str) {
        if self.cur_anim == name {
            return;
        }
        log_info!("动画: {} -> {}", self.cur_anim, name);
        self.cancel_move();
        self.cur_anim = name.to_string();
        if let Some(clip) = self.clips.get_mut(&self.cur_anim) {
            clip.seek(0);
        }
        self.frame_accum_ms = 0;
        self.anim_ended_fired = false;
        self.render_current();
    }

    pub fn render_current(&mut self) {
        let (dw, dh) = self.window_size();
        let pad = (state::PAD * self.scale).round() as usize;
        if let Some(clip) = self.clips.get_mut(&self.cur_anim) {
            let w = clip.webm.width as usize;
            let h = clip.webm.height as usize;
            let idx = clip.cur;
            if idx >= clip.frame_count() {
                return;
            }
            let frame = clip.next_frame(&mut self.color_dec, self.alpha_dec.as_mut(), &mut self.comp_buf);
            if let Some(frame) = frame {
                let dst_h = self.render_buf.len() / (w * 4);
                self.render_buf.fill(0);
                for y in 0..h {
                    if pad + y >= dst_h {
                        break;
                    }
                    let s = y * w * 4;
                    let d = (pad + y) * w * 4;
                    self.render_buf[d..d + w * 4].copy_from_slice(&frame[s..s + w * 4]);
                }
                // frame 借用结束；还原 cur（预览不推进）
                clip.cur = idx;
                self.win.resize(dw, dh);
                self.win.present(&self.render_buf, w, dst_h, self.facing_right);
            }
        }
    }

    fn on_anim_ended(&mut self) {
        let name = self.cur_anim.clone();
        let drag = self.cats.drag.clone().unwrap_or_default();
        if name == drag && self.dragging {
            if let Some(clip) = self.clips.get_mut(&self.cur_anim) {
                clip.seek(0);
            }
            self.anim_ended_fired = false;
            return;
        }
        if self.cats.turns.contains(&name) {
            self.facing_right = !self.facing_right;
        }
        if name == drag || self.cats.clicks.contains(&name) {
            // 交互动画 → 待机缓冲
            self.switch_anim(&self.cats.idle.clone().unwrap_or_default());
            return;
        }
        // 普通动画结束：若配置了动作间隔，先停顿再继续动画链
        if self.behavior.act_interval_ms > 0 {
            self.cooldown_until =
                Some(Instant::now() + Duration::from_millis(self.behavior.act_interval_ms));
            return;
        }
        self.pick_next_anim();
    }

    /// 动画链推进：先尝试规划移动，否则按配置占比选下一个动画。
    fn pick_next_anim(&mut self) {
        let can_move = self.try_plan_move();
        let next = state::pick_next(
            &self.cats,
            &self.cur_anim,
            self.no_move,
            can_move,
            self.behavior.idle_ratio,
            self.behavior.turn_ratio,
            self.behavior.act_ratio,
        );
        self.switch_anim(&next);
    }

    fn try_plan_move(&mut self) -> bool {
        if self.no_move || self.move_plan.is_some() {
            return false;
        }
        let (lx, _ly, rx, _ry) = self.avail_rect();
        let (wx, _) = self.window_size();
        let (x, _, x2, _) = self.win.get_rect();
        let cx = (x + x2) as f64 / 2.0;
        let dir_sign = if self.facing_right { 1.0 } else { -1.0 };
        let distance = self.behavior.move_min_px
            + (self.behavior.move_max_px - self.behavior.move_min_px) * state::rand_f64();
        let target_cx = cx + dir_sign * distance;
        let half_w = wx as f64 / 2.0;
        if target_cx < lx as f64 + self.behavior.move_margin_px + half_w
            || target_cx > rx as f64 - self.behavior.move_margin_px - half_w
        {
            return false;
        }
        let move_name = state::pick(&self.cats.moves, &self.cats.weights, None);
        let duration_ms = self.clips.get(&move_name).map(|c| c.duration_ms()).unwrap_or(2400);
        self.switch_anim(&move_name);
        self.move_plan = Some(MovePlan {
            start_x: x,
            target_x: (target_cx - half_w).round() as i32,
            y: self.win.get_rect().1,
            duration_ms,
        });
        self.move_accum_ms = 0;
        log_info!("开始移动: {} -> x={}", move_name, (target_cx - half_w).round() as i32);
        true
    }

    fn cancel_move(&mut self) {
        self.move_plan = None;
        self.move_accum_ms = 0;
    }

    pub fn go_corner(&mut self) {
        let (_lx, _ly, rx, ry) = self.avail_rect();
        let (wx, wh) = self.window_size();
        let x = rx - wx - state::CORNER_MARGIN.round() as i32;
        let y = ry - wh;
        self.win.move_to(x, y);
        // 位置保存由 App 统一处理
    }

    pub fn restore_position(&mut self, pc: &PetConfig) {
        let (lx, ly, rx, ry) = self.avail_rect();
        let (wx, wh) = self.window_size();
        match (pc.rx, pc.ry) {
            (Some(rxv), Some(ryv)) => {
                let x = lx + (rxv * (rx - lx) as f64) as i32 - wx / 2;
                let y = ly + (ryv * (ry - ly) as f64) as i32 - wh / 2;
                let x = x.clamp(lx, rx - wx);
                let y = y.clamp(ly, ry - wh);
                self.win.move_to(x, y);
            }
            _ => self.go_corner(),
        }
    }

    pub fn save_position(&mut self, pc: &mut PetConfig) {
        let (lx, ly, rx, ry) = self.avail_rect();
        let (x, y, x2, y2) = self.win.get_rect();
        let cx = (x + x2) as f64 / 2.0;
        let cy = (y + y2) as f64 / 2.0;
        if rx > lx && ry > ly {
            pc.rx = Some((cx - lx as f64) / (rx - lx) as f64);
            pc.ry = Some((cy - ly as f64) / (ry - ly) as f64);
        }
        pc.facing_right = self.facing_right;
        pc.scale = self.scale;
        pc.always_on_top = self.win_topmost;
        pc.no_move = self.no_move;
    }

    fn avail_rect(&self) -> (i32, i32, i32, i32) {
        crate::monitor::primary_work_area()
    }

    pub fn change_scale(&mut self, s: f64) {
        if (s - self.scale).abs() < 1e-6 {
            return;
        }
        log_info!("缩放: {} -> {}", self.scale, s);
        let old_bottom = self.win.get_rect().3;
        self.scale = s;
        let (wx, wh) = self.window_size();
        self.win.resize(wx, wh);
        self.win.move_to(self.win.get_rect().0, old_bottom - wh + 1);
        self.render_current();
    }

    pub fn set_no_move(&mut self, on: bool) {
        self.no_move = on;
        if on && self.move_plan.is_some() {
            self.switch_anim(&self.cats.idle.clone().unwrap_or_default());
        }
    }

    pub fn set_topmost(&mut self, on: bool) {
        self.win_topmost = on;
        self.win.set_topmost(on);
    }

    pub fn toggle_visible(&mut self) {
        if self.visible {
            self.win.hide();
            self.visible = false;
            log_info!("已隐藏");
        } else {
            self.win.show();
            self.visible = true;
            log_info!("已显示");
        }
    }

    // ---------------- 交互（平台无关事件入口，由桥接层调用） ----------------

    /// 鼠标按下（窗口客户区坐标，左上原点）。
    pub fn on_press(&mut self, cx: i32, cy: i32) {
        // 拖拽捕获由平台层负责（win32 SetCapture / AppKit 按下期间隐式捕获）
        let (wx, wy, _, _) = self.win.get_rect();
        let (sx, sy) = (wx + cx, wy + cy);
        self.press_global = Some((sx, sy));
        self.grab_offset = Some((sx - wx, sy - wy));
        self.dragging = false;
        self.cancel_move();
    }

    /// 鼠标移动（按下状态下）。用全局光标位置：鼠标可能已移出窗口边界。
    pub fn on_drag_move(&mut self) {
        if self.press_global.is_none() {
            return;
        }
        let (sx, sy) = cursor_pos();
        if sx == i32::MIN {
            return;
        }
        let (px, py) = self.press_global.unwrap();
        let dx = sx - px;
        let dy = sy - py;
        let dist = ((dx * dx + dy * dy) as f64).sqrt();
        if !self.dragging {
            if dist < state::DRAG_THRESHOLD * self.scale {
                return;
            }
            self.dragging = true;
            log_info!("开始拖拽");
            self.switch_anim(&self.cats.drag.clone().unwrap_or_default());
        }
        if let Some(off) = self.grab_offset {
            self.win.move_to(sx - off.0, sy - off.1);
        }
    }

    /// 鼠标抬起。
    pub fn on_release(&mut self) {
        let was_dragging = self.dragging;
        let (sx, sy) = cursor_pos();
        if was_dragging {
            self.just_dragged = true;
            log_info!("拖拽结束，位置 ({}, {})", sx, sy);
            if let Some(off) = self.grab_offset {
                self.win.move_to(sx - off.0, sy - off.1);
            }
            // 位置保存由 App 在拖拽结束后统一处理
            self.switch_anim(&self.cats.idle.clone().unwrap_or_default());
        } else {
            self.on_click();
        }
        self.dragging = false;
        self.press_global = None;
        self.grab_offset = None;
    }

    fn on_click(&mut self) {
        if self.just_dragged {
            self.just_dragged = false;
            return;
        }
        if self.cats.idle.as_deref() != Some(self.cur_anim.as_str()) {
            return;
        }
        self.cancel_move();
        let c = self.cats.clicks.clone();
        let pick = state::pick(&c, &self.cats.weights, None);
        log_info!("点击回应: {}", pick);
        self.switch_anim(&pick);
    }

    /// 说话气泡同步到平台窗口（仅 Windows；macOS 绘制时直接读 pet.bubble）。
    /// 窗口层按文本去重，文本未变时不重渲染，无每帧 GDI 开销。
    #[cfg(windows)]
    fn sync_bubble(&mut self) {
        match &self.bubble {
            Some(b) => self.win.set_bubble(&b.text),
            None => self.win.clear_bubble(),
        }
    }

    /// 每 tick（10ms）驱动：帧推进 + 移动插值。
    pub fn on_tick(&mut self) {
        // 外部指令（HTTP API）：高优先级，先于动画推进执行
        while let Some(cmd) = self.cmd_queue.pop_front() {
            self.exec_command(cmd);
        }
        // 气泡过期清理
        if let Some(b) = &self.bubble {
            if Instant::now() >= b.until {
                self.bubble = None;
            }
        }
        // 说话气泡同步到平台窗口：
        // Windows 由窗口层预渲染位图并在 present 时合成，需显式推送/清除；
        // macOS 绘制时直接读取 pet.bubble，无需同步（见 macos.rs draw_rect）。
        #[cfg(windows)]
        self.sync_bubble();

        let dt = self.last_tick.elapsed();
        self.last_tick = Instant::now();
        let dt_ms = dt.as_millis() as u64;
        if dt_ms == 0 {
            return;
        }

        let frame_ms = self
            .clips
            .get(&self.cur_anim)
            .map(|c| c.webm.frame_ms())
            .unwrap_or(state::FRAME_MS as u64);
        self.frame_accum_ms += dt_ms;
        if self.frame_accum_ms >= frame_ms {
            self.frame_accum_ms = 0;
            self.advance_frame();
        }

        // 动作间隔冷却到期 → 继续动画链
        if self.anim_ended_fired {
            if let Some(until) = self.cooldown_until {
                if Instant::now() >= until {
                    self.cooldown_until = None;
                    self.pick_next_anim();
                }
            }
        }

        if let Some(plan) = self.move_plan.clone() {
            self.move_accum_ms += dt_ms;
            let t = self.move_accum_ms as f64 / 1000.0;
            let dur = plan.duration_ms as f64 / 1000.0;
            let (lead, tail) = (state::MOVE_LEAD_SEC, state::MOVE_TAIL_SEC);
            let x = if t <= lead {
                plan.start_x as f64
            } else if t >= dur - tail {
                plan.target_x as f64
            } else {
                let progress = (t - lead) / (dur - lead - tail).max(0.1);
                plan.start_x as f64 + (plan.target_x - plan.start_x) as f64 * progress
            };
            self.win.move_to(x.round() as i32, plan.y);
            if t >= dur - tail {
                self.cancel_move();
            }
        }
    }

    /// 命中测试：全局坐标（屏幕）→ 按像素 alpha 判断是否可交互。
    pub fn hit_at(&self, gx: i32, gy: i32) -> bool {
        let (wx, wy, _, _) = self.win.get_rect();
        self.win.hit_test_alpha(gx - wx, gy - wy)
    }

    fn advance_frame(&mut self) {
        let (dw, dh) = self.window_size();
        let pad = (state::PAD * self.scale).round() as usize;
        let anim = self.cur_anim.clone();
        let (mut w, mut h) = (0usize, 0usize);
        let frame: Option<&[u8]> = match self.clips.get_mut(&anim) {
            Some(clip) => {
                // 先取尺寸（借用立即结束），再借可变 next_frame
                w = clip.webm.width as usize;
                h = clip.webm.height as usize;
                clip.next_frame(&mut self.color_dec, self.alpha_dec.as_mut(), &mut self.comp_buf)
            }
            None => None,
        };
        match frame {
            Some(frame) => {
                let dst_h = self.render_buf.len() / (w * 4);
                self.render_buf.fill(0);
                for y in 0..h {
                    if pad + y >= dst_h {
                        break;
                    }
                    let s = y * w * 4;
                    let d = (pad + y) * w * 4;
                    self.render_buf[d..d + w * 4].copy_from_slice(&frame[s..s + w * 4]);
                }
                self.win.resize(dw, dh);
                self.win.present(&self.render_buf, w, dst_h, self.facing_right);
            }
            None => {
                if !self.anim_ended_fired {
                    self.anim_ended_fired = true;
                    self.on_anim_ended();
                }
            }
        }
    }

    // ---------------- 菜单（数据驱动，平台渲染） ----------------

    /// 右键菜单数据。
    pub fn context_menu_items(&self) -> Vec<MenuEntry> {
        let mut items = vec![
            MenuEntry::item(MID_CORNER, "回到右下角"),
            MenuEntry::check(MID_ONTOP, "窗口置顶", self.win_topmost),
            MenuEntry::check(MID_NOMOVE, "不移动", self.no_move),
            MenuEntry::check(MID_AUTOSTART, "开机自启", crate::autostart::is_enabled()),
        ];
        let mut scales = Vec::new();
        for (i, s) in self.behavior.scale_steps.iter().enumerate() {
            let pct = (s * 100.0).round() as i32;
            scales.push(MenuEntry::check(
                MID_SCALE_BASE + i,
                &format!("{}%", pct),
                (self.scale - s).abs() < 0.02,
            ));
        }
        items.push(MenuEntry::separator());
        items.push(MenuEntry::submenu("大小", scales));
        items
    }

    /// 执行宠物自身命令（位置/置顶/不移动/大小）。全局命令（自启）由 App 处理。
    pub fn apply_command(&mut self, id: usize) {
        match id {
            MID_CORNER => self.go_corner(),
            MID_ONTOP => {
                let on = !self.win_topmost;
                self.set_topmost(on);
            }
            MID_NOMOVE => {
                let on = !self.no_move;
                self.set_no_move(on);
            }
            _ => {
                if id >= MID_SCALE_BASE && id < MID_SCALE_BASE + self.behavior.scale_steps.len() {
                    let i = id - MID_SCALE_BASE;
                    if i < self.behavior.scale_steps.len() {
                        self.change_scale(self.behavior.scale_steps[i]);
                    }
                }
            }
        }
    }
}
