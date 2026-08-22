//! 桌宠运行/几何常量与低级随机/加权抽取工具（平台无关）。
//! 行为决策（状态机）见 `behavior.rs`；本文件只保留渲染/交互所需的常量与工具。
#![allow(dead_code)]

use std::collections::HashMap;

// 几何常量
pub const CANVAS_W: f64 = 640.0;
pub const CANVAS_H: f64 = 360.0;
pub const PAD: f64 = 30.0; // 落地偏移：帧下移让脚踩窗口底线
pub const FRAME_MS: u32 = 40;

pub const DRAG_THRESHOLD: f64 = 5.0;
pub const CORNER_MARGIN: f64 = 24.0;

/// 从池中按权重随机选（排除 exclude；权重取 `weights` 中该键的值，缺省 1.0）。
/// 用于交互类（点击）选择；状态动作池的选择见 behavior::pick_action。
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

/// 当前本地时间（自 0 点起的分钟数），供状态机引擎判定状态。
pub fn now_minutes() -> u32 {
    use std::time::{SystemTime, UNIX_EPOCH};
    // 以当前时刻换算成当日分钟数；失败回退 0。
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let m = secs.rem_euclid(86400) / 60; // 本地时区近似；跨区由引擎按 HH:MM 规则匹配
    m as u32
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
