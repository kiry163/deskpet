// 宠物引擎：PIXI 渲染 + 动画链调度 + 移动计划（移植自 dsh-pet-standalone 行为模型，MIT）。
import { Application, AnimatedSprite, Container, Texture } from "pixi.js";
import {
  FramesMeta,
  loadAnimationTextures,
  releaseAnimationTextures,
} from "./assets";
import { Catalog, pick, pickNext } from "./catalog";

// 与 standalone state.rs 对齐
const MOVE_MIN_PX = 60;
const MOVE_MAX_PX = 240;
const MOVE_MARGIN = 100; // 人物半宽余量（640×360 画布人物约占中央 300px）
const MOVE_LEAD_SEC = 2.0;
const MOVE_TAIL_SEC = 2.0;
const DRAG_THRESHOLD = 5;
// 素材脚底在帧内 y=330/360，用 anchor 让脚底精确贴窗底
const FEET_ANCHOR_Y = 330 / 360;
// 640×360 画布内人物实际 bbox（原版 dsh-pet HIT_BOX，中心 x=320，脚底 y=330）：
// 拖拽边界按人物实际尺寸而非含透明区的画布尺寸计算，才能贴边
const PET_BBOX = { x0: 200, y0: 50, x1: 440, y1: 335 };

// 常驻动画上限（内存控制：640×360 RGBA ≈ 111MB/动画，最多缓存 3 个）
const MAX_CACHED_ANIMS = 3;

interface MovePlan {
  startX: number;
  targetX: number;
  durationMs: number;
  accumMs: number;
}

export interface EngineOptions {
  scale?: number;
  facingRight?: boolean;
  noMove?: boolean;
  workArea?: { width: number; height: number };
}

export class PetEngine {
  private app: Application;
  private catalog: Catalog;
  private meta: FramesMeta;
  private sprite: AnimatedSprite;
  private container: Container;
  private area: { width: number; height: number };

  // 纹理缓存（LRU）
  private cache = new Map<string, Texture[]>();
  private cacheOrder: string[] = [];

  curAnim = "";
  facingRight = false;
  noMove = false;
  petScale: number;

  private movePlan: MovePlan | null = null;
  private animEndedFired = false;

  // 交互状态
  private pressing = false;
  private dragging = false;
  private pressGlobalX = 0;
  private pressGlobalY = 0;
  private grabOffsetX = 0;
  private grabOffsetY = 0;

  constructor(
    app: Application,
    catalog: Catalog,
    meta: FramesMeta,
    opts: EngineOptions = {}
  ) {
    this.app = app;
    this.catalog = catalog;
    this.meta = meta;
    this.area = opts.workArea ?? { width: app.screen.width, height: app.screen.height };
    this.facingRight = opts.facingRight ?? false;
    this.noMove = opts.noMove ?? false;
    this.petScale = opts.scale ?? 0.72;
    this.container = new Container();
    this.sprite = new AnimatedSprite([Texture.EMPTY]); // 占位，switchAnim 时替换
    this.sprite.anchor.set(0.5, FEET_ANCHOR_Y); // 脚底锚点（落地对齐）
    this.sprite.animationSpeed = meta.fps / 60; // 12fps / 60fps tick
    this.sprite.loop = false;
    this.sprite.onComplete = () => this.onAnimEnded();
    // 初始位置：工作区水平居中，脚底贴工作区底
    this.sprite.position.set(this.area.width / 2, this.area.height);
    this.applyFacing();
    this.container.addChild(this.sprite);
    app.stage.addChild(this.container);
    app.ticker.add((ticker) => this.tick(ticker.deltaMS));
  }

  /** 设置缩放（0.5/0.72/0.85/1.0），sprite 缩放、脚底保持贴底 */
  setScale(scale: number): void {
    this.petScale = scale;
    this.applyFacing();
  }

  /** 可交互区域（stage 逻辑坐标包围盒），供穿透轮询 */
  getHitRegion(): { x: number; y: number; w: number; h: number } | null {
    if (!this.sprite.textures.length) return null;
    const b = this.sprite.getBounds();
    return { x: b.x, y: b.y, w: b.width, h: b.height };
  }

  // ---------------- 动画链 ----------------

  async switchAnim(name: string): Promise<void> {
    if (this.curAnim === name) return;
    this.cancelMove();
    this.curAnim = name;
    this.animEndedFired = false;
    const textures = await this.getTextures(name);
    this.sprite.textures = textures;
    this.sprite.loop = this.catalog.drag === name && this.dragging; // 拖拽中循环
    this.sprite.gotoAndPlay(0);
  }

  private async getTextures(name: string): Promise<Texture[]> {
    const hit = this.cache.get(name);
    if (hit) {
      // LRU 更新
      this.cacheOrder = this.cacheOrder.filter((n) => n !== name);
      this.cacheOrder.push(name);
      return hit;
    }
    const frameCount = this.meta.animations[name]?.frames ?? 121;
    const textures = await loadAnimationTextures(name, frameCount);
    // LRU：释放最久未用的，直到低于上限（idle 常驻不释放）
    this.cacheOrder = this.cacheOrder.filter((n) => n !== name);
    this.cache.set(name, textures);
    this.cacheOrder.push(name);
    while (this.cacheOrder.length > MAX_CACHED_ANIMS) {
      const victim = this.cacheOrder.shift()!;
      if (victim === this.catalog.idle) {
        this.cacheOrder.push(victim); // idle 放回队尾，跳过
        continue;
      }
      const t = this.cache.get(victim);
      if (t) {
        releaseAnimationTextures(t);
        this.cache.delete(victim);
      }
    }
    return textures;
  }

  private onAnimEnded(): void {
    if (this.animEndedFired) return;
    this.animEndedFired = true;
    const name = this.curAnim;

    if (name === this.catalog.drag && this.dragging) {
      // 拖拽中循环播放
      this.sprite.gotoAndPlay(0);
      this.animEndedFired = false;
      return;
    }
    if (name === this.catalog.turn) {
      this.facingRight = !this.facingRight;
      this.applyFacing();
    }
    if (name === this.catalog.drag || this.catalog.clicks.includes(name)) {
      void this.switchAnim(this.catalog.idle);
      return;
    }
    const canMove = this.tryPlanMove();
    const next = pickNext(this.catalog, name, this.noMove, canMove);
    void this.switchAnim(next);
  }

  // ---------------- 移动计划 ----------------

  private tryPlanMove(): boolean {
    if (this.noMove || this.movePlan) return false;
    const dir = this.facingRight ? 1 : -1;
    const x = this.sprite.position.x;
    const margin = MOVE_MARGIN + 20; // 人物半宽余量
    if (dir > 0 && x >= this.area.width - margin) return false;
    if (dir < 0 && x <= margin) return false;
    const dist = MOVE_MIN_PX + Math.random() * (MOVE_MAX_PX - MOVE_MIN_PX);
    const target = Math.max(
      MOVE_MARGIN,
      Math.min(this.area.width - MOVE_MARGIN, x + dir * dist)
    );
    const moveName = pick(this.catalog.moves);
    const durationMs = this.meta.animations[moveName]?.duration_ms ?? 2400;
    this.movePlan = { startX: x, targetX: target, durationMs, accumMs: 0 };
    void this.switchAnim(moveName);
    return true;
  }

  private cancelMove(): void {
    this.movePlan = null;
  }

  private tick(dtMs: number): void {
    // 帧推进由 AnimatedSprite 内部处理（animationSpeed），无需手动
    // 移动计划推进
    const plan = this.movePlan;
    if (plan) {
      plan.accumMs += dtMs;
      const t = plan.accumMs / 1000;
      const dur = plan.durationMs / 1000;
      let x: number;
      if (t <= MOVE_LEAD_SEC) {
        x = plan.startX;
      } else if (t >= dur - MOVE_TAIL_SEC) {
        x = plan.targetX;
      } else {
        const progress = (t - MOVE_LEAD_SEC) / Math.max(0.1, dur - MOVE_LEAD_SEC - MOVE_TAIL_SEC);
        x = plan.startX + (plan.targetX - plan.startX) * progress;
      }
      this.sprite.position.x = x;
      if (t >= dur - MOVE_TAIL_SEC) {
        this.cancelMove();
      }
    }
    // 拖拽跟手
    if (this.dragging) {
      // 位置由 pointermove 设置，这里仅保持
    }
  }

  // ---------------- 交互 ----------------

  /** 拖拽/点击边界（按人物实际 bbox 计算，可贴边） */
  private clampPos(x: number, y: number): { x: number; y: number } {
    const s = this.petScale;
    const halfW = ((PET_BBOX.x1 - PET_BBOX.x0) / 2) * s; // 人物半宽
    const above = (FEET_ANCHOR_Y * 360 - PET_BBOX.y0) * s; // 头顶到脚底锚点
    return {
      x: Math.max(halfW, Math.min(this.area.width - halfW, x)),
      y: Math.max(above, Math.min(this.area.height, y)),
    };
  }

  onPointerDown(clientX: number, clientY: number): void {
    this.pressing = true;
    this.dragging = false;
    this.pressGlobalX = clientX;
    this.pressGlobalY = clientY;
    this.grabOffsetX = this.sprite.position.x - clientX;
    this.grabOffsetY = this.sprite.position.y - clientY;
    this.cancelMove();
  }

  onPointerMove(clientX: number, clientY: number): void {
    if (!this.pressing) return;
    if (!this.dragging) {
      const dx = clientX - this.pressGlobalX;
      const dy = clientY - this.pressGlobalY;
      if (Math.hypot(dx, dy) > DRAG_THRESHOLD) {
        this.dragging = true;
        this.sprite.loop = true;
        void this.switchAnim(this.catalog.drag);
      }
    }
    if (this.dragging) {
      const p = this.clampPos(clientX + this.grabOffsetX, clientY + this.grabOffsetY);
      this.sprite.position.set(p.x, p.y);
    }
  }

  onPointerUp(): void {
    const wasDragging = this.dragging;
    this.pressing = false;
    this.dragging = false;
    this.sprite.loop = false;
    if (wasDragging) {
      void this.switchAnim(this.catalog.idle);
    } else {
      // 点击回应
      const click = pick(this.catalog.clicks);
      void this.switchAnim(click);
    }
  }

  private applyFacing(): void {
    const dir = this.facingRight ? -1 : 1;
    this.sprite.scale.set(dir * this.petScale, this.petScale);
  }

  /** 调试信息：渲染状态 */
  debugInfo(): string {
    const t = this.sprite.texture;
    const tex = this.sprite.textures;
    return [
      `anim=${this.curAnim}`,
      `textures=${tex.length}`,
      `cur=${this.sprite.currentFrame}`,
      `playing=${this.sprite.playing}`,
      `valid=${t.width > 0} w=${t.width} h=${t.height}`,
      `pos=(${this.sprite.position.x.toFixed(0)},${this.sprite.position.y.toFixed(0)})`,
      `scale.x=${this.sprite.scale.x}`,
      `visible=${this.sprite.visible}`,
      `cache=[${[...this.cache.keys()].join(",")}]`,
    ].join("  ");
  }

  destroy(): void {
    this.app.ticker.remove((ticker) => this.tick(ticker.deltaMS));
    for (const t of this.cache.values()) releaseAnimationTextures(t);
    this.cache.clear();
    this.container.destroy({ children: true });
  }
}
