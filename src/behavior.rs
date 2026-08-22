//! 状态机引擎（决策层，纯函数）：`时钟 × 状态集合配置 → current_state`。
//!
//! 设计（见 docs/行为状态机设计.md）：
//! - 状态集合**程序级、用户可编辑**：内置 `idle`/`active` + 默认预置 `lunch`，可自定义。
//! - 每个状态有两个**同级可调**触发维度：`time_rules[]`（固定时段，命中强制）+ `weight`（加权随机，0=不随机）。
//! - 裁决：命中时间规则 → 在命中集内按权重随机；自由时段 → 在 `weight>0` 的状态内按权重随机。无优先级。
//! - `interval_ms`（动作间隔）是**状态级**：在该状态下播完一个动作、等多久再播下一个。
//!
//! 本文件只做**决策**（纯函数、可单测），不碰渲染/播放；行为层（pet.rs）负责读 `current_state` 播状态池。
#![allow(dead_code)]

use serde::{Deserialize, Serialize};

/// 时间规则的进入方式。
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EnterMode {
    /// 到点立即进入（默认）。
    Instant,
    /// 顺延到下一个窗口边界才进入。
    NextWindow,
}

/// 时间规则的结束方式。
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExitMode {
    /// 到点立即退出（默认）。
    AtEnd,
    /// 顺延到下一个窗口边界才退出。
    NextWindow,
}

fn def_enter() -> EnterMode {
    EnterMode::Instant
}
fn def_exit() -> ExitMode {
    ExitMode::AtEnd
}

/// 一条固定时段规则：`start ~ end`（HH:MM），命中即**强制**进入该状态。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TimeRule {
    /// 开始时刻（HH:MM）。
    pub start: String,
    /// 结束时刻（HH:MM）。
    pub end: String,
    #[serde(default = "def_enter")]
    pub enter: EnterMode,
    #[serde(default = "def_exit")]
    pub exit: ExitMode,
}

impl TimeRule {
    /// 判断某分钟（自 0 点起的分钟数）是否在此规则覆盖范围内。
    pub fn covers(&self, now_min: u32) -> bool {
        let s = parse_hhmm(&self.start);
        let e = parse_hhmm(&self.end);
        match (s, e) {
            (Some(s), Some(e)) => {
                if s <= e {
                    now_min >= s && now_min <= e
                } else {
                    // 跨午夜：如 22:00 ~ 06:00
                    now_min >= s || now_min <= e
                }
            }
            _ => false,
        }
    }
}

/// 动作间隔（状态级）：播完一个动作后等待的毫秒范围；随机取其中一值。
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct IntervalMs {
    pub min_ms: u64,
    pub max_ms: u64,
}

impl Default for IntervalMs {
    fn default() -> Self {
        IntervalMs { min_ms: 2000, max_ms: 2000 }
    }
}

impl IntervalMs {
    /// 随机一个等待时长（min <= max）。
    pub fn random_ms(&self, rng: &mut dyn FnMut() -> f64) -> u64 {
        if self.max_ms <= self.min_ms {
            self.min_ms
        } else {
            self.min_ms + (rng() * ((self.max_ms - self.min_ms) as f64 + 1.0)) as u64
        }
    }
}

/// 一个状态的完整定义（程序级）。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StateDef {
    /// 状态 id（idle / active / lunch / 自定义）。
    pub id: String,
    /// 对外显示名（如「空闲」「活跃」「午休」）。
    pub name: String,
    #[serde(default = "def_true")]
    pub enabled: bool,
    /// 加权随机权重；`0` = 不参与随机（仅由时间规则进入）。
    #[serde(default)]
    pub weight: f64,
    /// 固定时段规则（可选）。
    #[serde(default)]
    pub time_rules: Vec<TimeRule>,
    /// 动作间隔（状态级）。
    #[serde(default)]
    pub interval: IntervalMs,
}

fn def_true() -> bool {
    true
}

/// 默认状态集合（内置 idle/active + 预置 lunch），见 docs/行为状态机设计.md §10.1。
pub fn default_states() -> Vec<StateDef> {
    vec![
        StateDef {
            id: "idle".to_string(),
            name: "空闲".to_string(),
            enabled: true,
            weight: 0.7,
            time_rules: vec![],
            interval: IntervalMs { min_ms: 5000, max_ms: 10000 },
        },
        StateDef {
            id: "active".to_string(),
            name: "活跃".to_string(),
            enabled: true,
            weight: 0.3,
            time_rules: vec![],
            interval: IntervalMs { min_ms: 2000, max_ms: 3000 },
        },
        StateDef {
            id: "lunch".to_string(),
            name: "午休".to_string(),
            enabled: true,
            weight: 0.0,
            time_rules: vec![TimeRule {
                start: "12:30".to_string(),
                end: "14:00".to_string(),
                enter: EnterMode::Instant,
                exit: ExitMode::AtEnd,
            }],
            interval: IntervalMs { min_ms: 5000, max_ms: 8000 },
        },
    ]
}

/// 状态机引擎（无状态；配置为 `states` 切片，决策为纯函数）。
pub struct Engine;

impl Engine {
    /// 判定当前状态：命中时间规则 → 在命中集内按权重随机；
    /// 否则（自由时段）→ 在 `weight>0` 且 enabled 的状态内按权重随机。
    ///
    /// `now_min` = 自 0 点起分钟数；`rng` 为随机源（缺省用 state::rand_f64）。
    pub fn pick_state<'a>(
        states: &'a [StateDef],
        now_min: u32,
        rng: &mut dyn FnMut() -> f64,
    ) -> Option<&'a StateDef> {
        let enabled: Vec<&StateDef> = states.iter().filter(|s| s.enabled).collect();
        let hits: Vec<&StateDef> = enabled
            .iter()
            .copied()
            .filter(|s| s.time_rules.iter().any(|r| r.covers(now_min)))
            .collect();
        if !hits.is_empty() {
            return weighted_pick(&hits, rng);
        }
        let pool: Vec<&StateDef> = enabled
            .iter()
            .copied()
            .filter(|s| s.weight > 0.0)
            .collect();
        weighted_pick(&pool, rng)
    }
}

/// 按权重从状态池里选一个（未命中/权重全 0 → 取第一个）。
fn weighted_pick<'a>(pool: &[&'a StateDef], rng: &mut dyn FnMut() -> f64) -> Option<&'a StateDef> {
    if pool.is_empty() {
        return None;
    }
    let total: f64 = pool.iter().map(|s| s.weight.max(0.0)).sum();
    if total <= 0.0 {
        return Some(pool[0]);
    }
    let mut r = rng() * total;
    for s in pool {
        r -= s.weight.max(0.0);
        if r < 0.0 {
            return Some(s);
        }
    }
    Some(*pool.last().unwrap())
}

/// 解析 "HH:MM" → 自 0 点起的分钟数；非法返回 None。
fn parse_hhmm(s: &str) -> Option<u32> {
    let mut it = s.trim().split(':');
    let h: u32 = it.next()?.trim().parse().ok()?;
    let m: u32 = it.next()?.trim().parse().ok()?;
    Some(h * 60 + m)
}

/// 从动作集（name → weight）按权重随机选一个；排除 exclude（用于避免连续重复）。
pub fn pick_action<'a>(
    names: &'a [String],
    weights: &std::collections::HashMap<String, f64>,
    exclude: Option<&str>,
    rng: &mut dyn FnMut() -> f64,
) -> Option<&'a String> {
    if names.is_empty() {
        return None;
    }
    let candidates: Vec<&String> = names
        .iter()
        .filter(|n| Some(n.as_str()) != exclude)
        .collect();
    let pick_from: Vec<&String> = if candidates.is_empty() {
        names.iter().collect()
    } else {
        candidates
    };
    let w = |n: &String| weights.get(n.as_str()).copied().unwrap_or(1.0).max(0.0);
    let total: f64 = pick_from.iter().map(|n| w(n)).sum();
    if total <= 0.0 {
        return Some(pick_from[0]);
    }
    let mut r = rng() * total;
    for n in &pick_from {
        r -= w(n);
        if r < 0.0 {
            return Some(n);
        }
    }
    Some(*pick_from.last().unwrap())
}

pub fn rand_f64() -> f64 {
    crate::state::rand_f64()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn const_rng(x: f64) -> impl FnMut() -> f64 {
        move || x
    }

    fn min(from: u32, to: u32) -> u32 {
        from * 60 + to
    }

    #[test]
    fn lunch_forced_in_time_window() {
        let states = default_states();
        // 12:30 → 命中午休时间规则（即使 weight=0）
        let s = Engine::pick_state(&states, min(13, 0), &mut const_rng(0.0));
        assert_eq!(s.map(|x| x.id.as_str()), Some("lunch"));
    }

    #[test]
    fn free_period_picks_weighted_among_positive() {
        let states = default_states();
        // 10:00 无时间规则命中 → 在 weight>0 (idle 0.7 / active 0.3) 里按权重
        let s = Engine::pick_state(&states, min(10, 0), &mut const_rng(0.0));
        assert_eq!(s.map(|x| x.id.as_str()), Some("idle"));
    }

    #[test]
    fn zero_weight_only_via_time_rule() {
        // 把 idle/active 都关掉，只剩 lunch（weight=0），自由时段不该选它
        let states = vec![
            StateDef { id: "a".into(), name: "a".into(), enabled: false, weight: 0.0, time_rules: vec![], interval: Default::default() },
            StateDef { id: "b".into(), name: "b".into(), enabled: true, weight: 0.0, time_rules: vec![TimeRule { start: "12:30".into(), end: "14:00".into(), enter: EnterMode::Instant, exit: ExitMode::AtEnd }], interval: Default::default() },
        ];
        // 自由时段（10:00）→ 无 weight>0 候选 → None
        assert!(Engine::pick_state(&states, min(10, 0), &mut const_rng(0.0)).is_none());
        // 命中 13:00 → b（weight=0 但仍被时间规则强制）
        assert_eq!(Engine::pick_state(&states, min(13, 0), &mut const_rng(0.0)).map(|x| x.id.as_str()), Some("b"));
    }

    #[test]
    fn interval_random_within_range() {
        let i = IntervalMs { min_ms: 2000, max_ms: 3000 };
        let v = i.random_ms(&mut const_rng(0.5));
        assert!((2000..=3000).contains(&v));
        assert_eq!(IntervalMs::default().random_ms(&mut const_rng(1.0)), 2000);
    }

    #[test]
    fn time_rule_covers_normal_and_wrap() {
        let r = TimeRule { start: "12:30".into(), end: "14:00".into(), enter: EnterMode::Instant, exit: ExitMode::AtEnd };
        assert!(r.covers(min(12, 45)));
        assert!(!r.covers(min(11, 0)));
        let wrap = TimeRule { start: "22:00".into(), end: "06:00".into(), enter: EnterMode::Instant, exit: ExitMode::AtEnd };
        assert!(wrap.covers(min(23, 0)));
        assert!(wrap.covers(min(3, 0)));
        assert!(!wrap.covers(min(12, 0)));
    }
}
