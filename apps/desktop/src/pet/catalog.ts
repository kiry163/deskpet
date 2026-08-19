// 动画目录 + 动画链状态机（移植自 ianlike-ui/dsh-pet-standalone 的 state.rs，MIT）。
// 动画链：30% 待机 / 40% 转向 / 80% 动作 / 移动分支。

import type { FramesMeta } from "./assets";

export interface Catalog {
  idle: string;
  turn: string;
  moves: string[];
  clicks: string[];
  drag: string;
  acts: string[];
  names: string[];
}

// 与 standalone 一致：这些动画不进入 acts 池
const SPECIAL = [
  "待机呼吸休闲",
  "东张西望",
  "螃蟹走路",
  "原地漂浮踏步",
  "原地左转奔跑",
  "点击回应 - 开心跃动",
  "点击回应 - 害羞惊讶",
  "点击回应 - 傲娇生气（侧身展示）",
  "被鼠标拖拽悬空反馈",
];

export function buildCatalog(meta: FramesMeta): Catalog {
  const names = Object.keys(meta.animations);
  const idle = "待机呼吸休闲";
  const turn = "东张西望";
  const moves = ["螃蟹走路", "原地漂浮踏步", "原地左转奔跑"];
  const clicks = [
    "点击回应 - 开心跃动",
    "点击回应 - 害羞惊讶",
    "点击回应 - 傲娇生气（侧身展示）",
  ];
  const drag = "被鼠标拖拽悬空反馈";
  const acts = names.filter((n) => !SPECIAL.includes(n));
  return { idle, turn, moves, clicks, drag, acts, names };
}

export function pick(pool: string[], exclude?: string): string {
  if (pool.length === 0) return "";
  let idx = Math.floor(Math.random() * pool.length);
  if (exclude !== undefined && pool.length > 1 && pool[idx] === exclude) {
    idx = (idx + 1) % pool.length;
  }
  return pool[idx];
}

export function pickNext(
  catalog: Catalog,
  current: string,
  noMove: boolean,
  canMove: boolean
): string {
  const roll = Math.random();
  if (roll < 0.3) return catalog.idle;
  if (roll < 0.4) return catalog.turn;
  if (roll < 0.8) return pick(catalog.acts, current);
  // 移动分支
  if (!noMove && canMove) return pick(catalog.moves);
  return pick(catalog.acts, current);
}
