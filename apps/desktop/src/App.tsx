import { useEffect, useRef } from "react";
import { Application } from "pixi.js";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { initAssets } from "./pet/assets";
import { buildCatalog } from "./pet/catalog";
import { PetEngine } from "./pet/engine";
import "./App.css";

interface PetConfigJson {
  scale: number;
  no_move: boolean;
  always_on_top: boolean;
  autostart: boolean;
  facing_right: boolean;
  x: number | null;
  y: number | null;
}

interface WorkArea {
  width: number;
  height: number;
  x: number;
  y: number;
  scaleFactor: number;
}

export default function App() {
  const hostRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    let cancelled = false;
    let engine: PetEngine | null = null;
    let regionTimer: number | undefined;
    const unlisteners: (() => void)[] = [];

    (async () => {
      try {
        const { meta } = await initAssets();
        if (cancelled) return;

        // 工作区（overlay 窗口铺满主屏，桌宠可自由移动到任意位置）
        const area = await invoke<WorkArea>("get_work_area");

        const app = new Application();
        await app.init({
          width: area.width,
          height: area.height,
          backgroundAlpha: 0,
          antialias: true,
          resolution: window.devicePixelRatio || 1,
          autoDensity: true,
        });
        if (cancelled) {
          app.destroy(true);
          return;
        }
        hostRef.current!.appendChild(app.canvas);

        // 加载配置并创建引擎
        const cfg = await invoke<PetConfigJson>("get_config");
        const catalog = buildCatalog(meta);
        engine = new PetEngine(app, catalog, meta, {
          scale: cfg.scale,
          facingRight: cfg.facing_right,
          noMove: cfg.no_move,
          workArea: { width: area.width, height: area.height },
        });
        void engine.switchAnim(catalog.idle);

        // 托盘事件：缩放 / 不移动
        unlisteners.push(
          await listen<number>("pet-scale", (e) => engine!.setScale(e.payload))
        );
        unlisteners.push(
          await listen<boolean>("pet-no-move", (e) => {
            engine!.noMove = e.payload;
          })
        );

        // 穿透轮询：低频上报桌宠包围盒（Rust 侧据此切换点击穿透）
        regionTimer = window.setInterval(() => {
          if (!engine) return;
          const b = engine.getHitRegion();
          if (b) {
            void invoke("update_hit_region", {
              x: b.x,
              y: b.y,
              w: b.w,
              h: b.h,
            });
          }
        }, 200);

        // 交互：点击回应 / 拖拽（x/y 双向，可拖到屏幕任意位置）
        // 拖拽期间上报全窗口可交互（避免光标移出包围盒导致穿透、拖拽卡死）
        const canvas = app.canvas;
        const toLocal = (e: PointerEvent) => {
          const r = canvas.getBoundingClientRect();
          return { x: e.clientX - r.left, y: e.clientY - r.top };
        };
        canvas.addEventListener("pointerdown", (e) => {
          void invoke("update_hit_region", {
            x: 0,
            y: 0,
            w: area.width,
            h: area.height,
          });
          canvas.style.cursor = "grabbing";
          const p = toLocal(e);
          engine!.onPointerDown(p.x, p.y);
        });
        canvas.addEventListener("pointermove", (e) => {
          // 能收到 move 说明窗口未穿透（光标在桌宠身上）→ 手势反馈
          canvas.style.cursor = "pointer";
          const p = toLocal(e);
          engine!.onPointerMove(p.x, p.y);
        });
        canvas.addEventListener("pointerup", () => {
          canvas.style.cursor = "pointer";
          engine!.onPointerUp();
        });
        canvas.addEventListener("pointerleave", () => {
          canvas.style.cursor = "default";
          engine!.onPointerUp();
        });
      } catch (err) {
        console.error("deskpet init failed:", err);
      }
    })();

    return () => {
      cancelled = true;
      if (regionTimer) window.clearInterval(regionTimer);
      unlisteners.forEach((u) => u());
      engine?.destroy();
    };
  }, []);

  return (
    <div
      style={{
        position: "relative",
        width: "100vw",
        height: "100vh",
        background: "transparent",
        overflow: "hidden",
      }}
    >
      <div ref={hostRef} style={{ width: "100%", height: "100%" }} />
    </div>
  );
}
