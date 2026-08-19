// 素材访问：通过 Tauri asset protocol 读取工作区 assets/frames（dev/prod 一致）。
// 开发期 framesDir 由 Rust 命令返回；正式版改为应用数据目录（外置素材包）。
import { invoke } from "@tauri-apps/api/core";
import { convertFileSrc } from "@tauri-apps/api/core";
import { Assets, Texture } from "pixi.js";

export interface AnimMeta {
  frames: number;
  duration_ms: number;
  bytes: number;
}

export interface FramesMeta {
  version: number;
  fps: number;
  width: number;
  height: number;
  source: string;
  animations: Record<string, AnimMeta>;
}

let framesDir: string | null = null;

export async function initAssets(): Promise<{ meta: FramesMeta; framesDir: string }> {
  const cfg = (await invoke("get_assets_config")) as { framesDir: string };
  framesDir = cfg.framesDir;
  const meta = await fetch(convertFileSrc(`${framesDir}/meta.json`)).then((r) =>
    r.json()
  );
  return { meta: meta as FramesMeta, framesDir };
}

/** 单个帧的 asset URL（PIXI Assets.load 用） */
function frameUrl(anim: string, index: number): string {
  const name = `f${String(index).padStart(5, "0")}.webp`;
  return convertFileSrc(`${framesDir}/${anim}/${name}`);
}

/** 动画帧纹理（并发加载）。 */
export async function loadAnimationTextures(
  anim: string,
  frameCount: number
): Promise<Texture[]> {
  const urls = Array.from({ length: frameCount }, (_, i) => frameUrl(anim, i));
  return Promise.all(urls.map((u) => Assets.load<Texture>(u)));
}

/** 释放动画纹理（frameCount 由 meta 提供）。 */
export function releaseAnimationTextures(textures: Texture[]): void {
  for (const t of textures) {
    t.destroy(true);
  }
}
