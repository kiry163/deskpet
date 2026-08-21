//! 动画目录 + 动态分类 + 动画链状态机（1:1 移植上游 catalog.py / window.py）。
#![allow(dead_code)]

use std::collections::HashMap;

// 几何常量
pub const CANVAS_W: f64 = 640.0;
pub const CANVAS_H: f64 = 360.0;
pub const PAD: f64 = 30.0; // 落地偏移：帧下移让脚踩窗口底线
pub const FRAME_MS: u32 = 40;

// 移动插值时间（秒）
pub const MOVE_LEAD_SEC: f64 = 2.0;
pub const MOVE_TAIL_SEC: f64 = 2.0;

pub const DRAG_THRESHOLD: f64 = 5.0;
pub const CORNER_MARGIN: f64 = 24.0;

// 触发类型（与 db.rs TRIGGERS 对齐；素材包无 manifest 后由管理端配置）
pub const TRIGGER_IDLE: &str = "idle";
pub const TRIGGER_TURN: &str = "turn";
pub const TRIGGER_MOVE: &str = "move";
pub const TRIGGER_CLICK: &str = "click";
pub const TRIGGER_DRAG: &str = "drag";
pub const TRIGGER_IDLE_ACT: &str = "idle_act";

/// 动态分类结果。
pub struct Category {
    pub idle: Option<String>,
    pub turn: Option<String>,
    pub idles: Vec<String>,
    pub turns: Vec<String>,
    pub moves: Vec<String>,
    pub clicks: Vec<String>,
    pub drag: Option<String>,
    pub acts: Vec<String>,
    /// 动画名 → 权重（桌宠级配置；pick 按权重随机，缺省 1.0）。
    pub weights: HashMap<String, f64>,
}

impl Category {
    pub fn empty() -> Category {
        Category {
            idle: None,
            turn: None,
            idles: Vec::new(),
            turns: Vec::new(),
            moves: Vec::new(),
            clicks: Vec::new(),
            drag: None,
            acts: Vec::new(),
            weights: HashMap::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.idles.is_empty() && self.turns.is_empty()
            && self.moves.is_empty() && self.clicks.is_empty()
            && self.drag.is_none() && self.acts.is_empty()
    }
}

/// 由管理端动作配置（action → (trigger, weight, enabled)）构建动态分类。
/// - 未登记动画默认 `idle_act`（闲时随机池）；enabled=false 的动画不入任何池；
/// - 触发类型映射：idle→待机 / turn→转向 / move→移动 / click→点击 / drag→拖拽（取第一个）；
///   其余（含 idle_act 与未知）→ 闲时随机池；
/// - 权重（weight）随分类存入 `cat.weights`（pick 按权重随机，缺省 1.0）；
/// - 兜底：无待机动画时取第一个启用动画作为待机（保证桌宠能安静待机）。
pub fn build_categories_from_actions(
    names: &[String],
    actions: &HashMap<String, (String, f64, bool)>,
) -> Category {
    let mut cat = Category::empty();
    for name in names {
        let (trigger, weight, enabled) = actions
            .get(name)
            .cloned()
            .unwrap_or_else(|| (TRIGGER_IDLE_ACT.to_string(), 1.0, true));
        cat.weights.insert(name.clone(), weight.max(0.0));
        if !enabled {
            continue;
        }
        match trigger.as_str() {
            TRIGGER_IDLE => cat.idles.push(name.clone()),
            TRIGGER_TURN => cat.turns.push(name.clone()),
            TRIGGER_MOVE => cat.moves.push(name.clone()),
            TRIGGER_CLICK => cat.clicks.push(name.clone()),
            TRIGGER_DRAG => {
                if cat.drag.is_none() {
                    cat.drag = Some(name.clone());
                }
            }
            _ => cat.acts.push(name.clone()),
        }
    }
    cat.idle = cat.idles.first().cloned();
    cat.turn = cat.turns.first().cloned();
    if cat.idles.is_empty() {
        let fallback = cat.acts.first().cloned().or_else(|| names.first().cloned());
        if let Some(f) = fallback {
            cat.idles.push(f.clone());
            cat.idle = Some(f);
        }
    }
    cat
}

/// 从池中按权重随机选（排除 exclude；权重取 `weights` 中该动画的值，缺省 1.0）。
pub fn pick(pool: &[String], weights: &HashMap<String, f64>, exclude: Option<&str>) -> String {
    if pool.is_empty() {
        return String::new();
    }
    let mut candidates: Vec<&String> = pool
        .iter()
        .filter(|n| Some(n.as_str()) != exclude)
        .collect();
    if candidates.is_empty() {
        candidates = pool.iter().collect();
    }
    let w = |n: &str| weights.get(n).copied().unwrap_or(1.0).max(0.0);
    let total: f64 = candidates.iter().map(|n| w(n)).sum();
    if total <= 0.0 {
        return candidates[0].clone();
    }
    let mut r = rand_f64() * total;
    for n in &candidates {
        r -= w(n);
        if r < 0.0 {
            return (*n).clone();
        }
    }
    candidates.last().unwrap().to_string()
}

/// 动画链：按配置占比分派 待机 / 转向 / 闲时动作 / 移动。
/// `idle_ratio <= turn_ratio <= act_ratio <= 1`（调用方已归一化）；
/// no_move 时移动概率并入动作；待机/转向支持多视频。
pub fn pick_next(
    cat: &Category,
    current: &str,
    no_move: bool,
    can_move: bool,
    idle_ratio: f64,
    turn_ratio: f64,
    act_ratio: f64,
) -> String {
    let roll = rand_f64();
    if roll < idle_ratio {
        if !cat.idles.is_empty() {
            return pick(&cat.idles, &cat.weights, Some(current));
        }
        return pick(&cat.acts, &cat.weights, Some(current));
    }
    if roll < turn_ratio {
        if !cat.turns.is_empty() {
            return pick(&cat.turns, &cat.weights, Some(current));
        }
        return pick(&cat.acts, &cat.weights, Some(current));
    }
    if roll < act_ratio {
        return pick(&cat.acts, &cat.weights, Some(current));
    }
    // 移动分支
    if !no_move && can_move && !cat.moves.is_empty() {
        return pick(&cat.moves, &cat.weights, None);
    }
    pick(&cat.acts, &cat.weights, Some(current))
}

// ---- 简易随机（xorshift + 时间种子，零依赖）----
use std::sync::atomic::{AtomicU64, Ordering};

static SEED: AtomicU64 = AtomicU64::new(0x9E3779B97F4A7C15);

fn next_rand() -> u64 {
    let mut x = SEED.load(Ordering::Relaxed);
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    SEED.store(x, Ordering::Relaxed);
    x
}

pub fn rand_f64() -> f64 {
    (next_rand() >> 11) as f64 / (1u64 << 53) as f64
}

/// 用系统时间做种子，避免每次启动动画序列相同。
pub fn init_random() {
    use std::time::{SystemTime, UNIX_EPOCH};
    if let Ok(d) = SystemTime::now().duration_since(UNIX_EPOCH) {
        let t = d.as_nanos() as u64;
        SEED.store(t ^ 0x9E3779B97F4A7C15, Ordering::Relaxed);
    }
}
