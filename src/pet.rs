//! 单只桌宠：窗口 + 动画状态机 + 交互 + 渲染（平台无关核心）。
//! 平台差异（窗口创建/事件来源/菜单渲染/光标）由 win32.rs / macos.rs 桥接层处理。
#![allow(non_snake_case, dead_code)]

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use crate::assets::RoleAssets;
use crate::behavior::{self, StateDef};
use crate::clip::{ClipDecoder, H, W};
use crate::config::PetConfig;
use crate::db::ActionRow;
use crate::menu::MenuEntry;
use crate::platform::{cursor_pos, PetWindow};
use crate::state;

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

/// 行为引擎参数（来自 PetConfig；取决于最终设计：仅缩放档位；状态/间隔在状态集合里）。
#[derive(Clone, Debug)]
pub struct Behavior {
    /// 缩放档位（大小菜单/设置页）。
    pub scale_steps: Vec<f64>,
}

impl From<&PetConfig> for Behavior {
    fn from(pc: &PetConfig) -> Behavior {
        Behavior {
            scale_steps: pc.scale_steps.clone(),
        }
    }
}

/// 宠物行为池：按状态划分的动作池（加权）+ 交互类动作 + 体型基准。
#[derive(Clone)]
pub struct StatePools {
    /// state_id → [(动作名, 该状态下权重)]
    pub pools: HashMap<String, Vec<(String, f64)>>,
    /// 点击回应动作池
    pub clicks: Vec<String>,
    /// 拖拽反馈动作（取第一个）
    pub drag: Option<String>,
    /// 体型基准动作（待机）
    pub idle: Option<String>,
}

impl StatePools {
    /// 取某状态的动作池（缺省为空）。
    pub fn pool(&self, state_id: &str) -> &[(String, f64)] {
        self.pools.get(state_id).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// 确保存在 id 状态池（缺省归入空闲池并启用，保证桌宠会动）。
    pub fn ensure_idle_pool(&mut self) {
        if !self.pools.contains_key("idle") {
            if let Some(idle) = self.idle.clone() {
                self.pools.insert("idle".to_string(), vec![(idle, 1.0)]);
            }
        }
    }
}

/// 由管理端动作配置构建行为池（最终模型）。
/// - `actions`：pet_actions 行（交互 click/drag 或 state）。
/// - `action_states`：(action, state_id, weight, enabled)，仅 state 类有效。
/// - `names`：该宠物素材目录内全部 webm 名（未登记默认归入空闲池并启用）。
pub fn build_pools(
    names: &[String],
    actions: &[ActionRow],
    action_states: &[(String, String, f64, bool)],
) -> StatePools {
    let mut pools: HashMap<String, Vec<(String, f64)>> = HashMap::new();
    let mut clicks = Vec::new();
    let mut drag = None;

    // 动作 → 行 索引
    let row_of: HashMap<&str, &ActionRow> =
        actions.iter().map(|a| (a.action.as_str(), a)).collect();

    // 动作 → 状态绑定（多选加权）
    let mut bindings: HashMap<&str, Vec<(String, f64, bool)>> = HashMap::new();
    for (action, state_id, weight, enabled) in action_states {
        bindings.entry(action.as_str()).or_default().push((state_id.clone(), *weight, *enabled));
    }

    for name in names {
        let row = row_of.get(name.as_str()).copied();
        let owner_kind = row.map(|r| r.owner_kind.as_str()).unwrap_or("state");
        let enabled = row.map(|r| r.enabled).unwrap_or(true);
        if !enabled {
            continue;
        }
        match owner_kind {
            "interactive" => {
                let kind = row.and_then(|r| r.kind.as_deref()).unwrap_or("");
                if kind == "drag" && drag.is_none() {
                    drag = Some(name.clone());
                } else if kind == "click" {
                    clicks.push(name.clone());
                } else {
                    // 未知交互类型 → 当作点击
                    clicks.push(name.clone());
                }
            }
            _ => {
                // state 类：按 action_states 归属；无绑定默认归空闲池
                let binds = bindings.get(name.as_str()).cloned().unwrap_or_else(|| vec![("idle".to_string(), 1.0, true)]);
                for (state_id, weight, st_enabled) in binds {
                    if st_enabled {
                        pools.entry(state_id.clone()).or_default().push((name.clone(), weight));
                    }
                }
            }
        }
    }

    let idle = pools.get("idle").and_then(|v| v.first().map(|(n, _)| n.clone()));
    let mut sp = StatePools { pools, clicks, drag, idle };
    sp.ensure_idle_pool();
    sp.idle = sp.pools.get("idle").and_then(|v| v.first().map(|(n, _)| n.clone()));
    sp
}

pub struct Pet {
    pub win: PetWindow,
    pub pools: StatePools,
    /// 程序级状态集合（状态机配置）。
    pub states: Vec<StateDef>,
    /// 当前状态 id（由状态机引擎维护）。
    pub current_state: String,
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

    /// 行为引擎参数（程序级，可热更新；移动/缩放）。
    pub behavior: Behavior,
    /// 动作间隔冷却（状态动作结束后等待；None = 无需等待）。
    cooldown_until: Option<Instant>,

    last_tick: Instant,
}

/// 从素材集 + 管理端动作配置构建解码器 + 行为池。
fn build_from_role(
    role: &RoleAssets,
    pools: StatePools,
    states: &[StateDef],
) -> HashMap<String, ClipDecoder> {
    let mut clips: HashMap<String, ClipDecoder> = HashMap::new();
    for (name, wm) in &role.videos {
        if let Some(dec) = ClipDecoder::new(wm.clone()) {
            clips.insert(name.clone(), dec);
        }
    }
    log_info!(
        "行为池: idle={:?} clicks={} drag={:?} 状态池={:?} 状态数={}",
        pools.idle,
        pools.clicks.len(),
        pools.drag,
        pools.pools.len(),
        states.len()
    );
    clips
}

impl Pet {
    pub fn new(
        win: PetWindow,
        role: &RoleAssets,
        pc: &PetConfig,
        pools: StatePools,
        states: &[StateDef],
    ) -> Pet {
        let clips = build_from_role(role, pools.clone(), states);
        let mut pet = Pet {
            win,
            pools,
            states: states.to_vec(),
            current_state: "idle".to_string(),
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
            behavior: Behavior::from(pc),
            cooldown_until: None,
            last_tick: Instant::now(),
        };
        let (w, h) = pet.window_size();
        pet.win.resize(w, h);
        pet.win.set_topmost(pc.always_on_top);
        let idle = pet.pools.idle.clone().unwrap_or_default();
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

    /// 热替换素材集（导入后 / 角色切换）：重建 clips/行为池，重播待机动画，保留窗口与位置。
    pub fn swap_role(&mut self, role: &RoleAssets, pools: StatePools, states: &[StateDef]) {
        let clips = build_from_role(role, pools.clone(), states);
        self.clips = clips;
        self.pools = pools;
        self.states = states.to_vec();
        self.current_state = "idle".to_string();
        self.cur_anim = String::new();
        self.anim_ended_fired = false;
        self.cooldown_until = None;
        let idle = self.pools.idle.clone().unwrap_or_default();
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

    /// 动作名解析：精确名 → 语义名（idle/click/drag）→ 子串最近匹配。
    pub fn resolve_action(&self, query: &str) -> Option<String> {
        let q = query.trim();
        if q.is_empty() {
            return None;
        }
        if self.clips.contains_key(q) {
            return Some(q.to_string());
        }
        let sem = match q.to_ascii_lowercase().as_str() {
            "idle" => self.pools.idle.clone(),
            "click" | "clicks" => self.pools.clicks.first().cloned(),
            "drag" => self.pools.drag.clone(),
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
        self.win.move_to(tx, ty);
        log_info!("指令 move: ({:.2}, {:.2}) -> ({}, {})", x, y, tx, ty);
    }

    pub fn switch_anim(&mut self, name: &str) {
        if self.cur_anim == name {
            return;
        }
        log_info!("动画: {} -> {}", self.cur_anim, name);
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
        let drag = self.pools.drag.clone().unwrap_or_default();
        if name == drag && self.dragging {
            // 拖拽中：循环播放拖拽动作
            if let Some(clip) = self.clips.get_mut(&self.cur_anim) {
                clip.seek(0);
            }
            self.anim_ended_fired = false;
            return;
        }
        if name == drag || self.pools.clicks.contains(&name) {
            // 交互动画结束 → 回到当前状态的自主循环（不改变状态）
            self.play_from_current_state();
            return;
        }
        // 状态动作结束：按「当前状态的间隔」停顿后再继续
        let interval = self.current_state_interval_ms();
        if interval > 0 {
            self.cooldown_until = Some(Instant::now() + Duration::from_millis(interval));
            return;
        }
        self.pick_next_anim();
    }

    /// 动画链推进：重算当前状态（支持时段切换）→ 从该状态动作池加权随机选下一个。
    fn pick_next_anim(&mut self) {
        self.current_state = behavior::Engine::pick_state(
            &self.states,
            state::now_minutes(),
            &mut || behavior::rand_f64(),
        )
        .map(|s| s.id.clone())
        .unwrap_or_else(|| "idle".to_string());
        self.play_from_current_state();
    }

    /// 从当前状态动作池选一个动作播放（空则回退空闲池）。
    fn play_from_current_state(&mut self) {
        let pool = self.pools.pool(&self.current_state).to_vec();
        if pool.is_empty() {
            let _ = self.play_from_pool(&self.pools.pool("idle").to_vec());
            return;
        }
        let _ = self.play_from_pool(&pool);
    }

    fn play_from_pool(&mut self, pool: &[(String, f64)]) -> bool {
        if pool.is_empty() {
            return false;
        }
        let names: Vec<String> = pool.iter().map(|(n, _)| n.clone()).collect();
        let weights: HashMap<String, f64> = pool.iter().map(|(n, w)| (n.clone(), *w)).collect();
        let next = behavior::pick_action(
            &names,
            &weights,
            Some(&self.cur_anim),
            &mut || behavior::rand_f64(),
        )
        .cloned()
        .unwrap_or_else(|| names[0].clone());
        self.switch_anim(&next);
        true
    }

    /// 当前状态的动作间隔（状态级，随机取范围内一值）。
    fn current_state_interval_ms(&self) -> u64 {
        self.states
            .iter()
            .find(|s| s.id == self.current_state)
            .map(|s| s.interval.random_ms(&mut || behavior::rand_f64()))
            .unwrap_or(0)
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
            self.switch_anim(&self.pools.drag.clone().unwrap_or_default());
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
            self.play_from_current_state();
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
        // 仅在处于待机（体型基准）状态时响应点击
        if self.pools.idle.as_deref() != Some(self.cur_anim.as_str()) {
            return;
        }
        let c = self.pools.clicks.clone();
        let pick = state::pick(&c, &HashMap::new(), None);
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
