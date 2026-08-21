// deskpet 控制台 API 封装：统一 {ok, data?, error?} 响应；导入走裸 zip body。

export interface ApiResp<T = unknown> {
  ok: boolean
  data?: T
  error?: string
}

export interface PetState {
  anim: string | null
  x: number
  y: number
  w: number
  h: number
  facing_right: boolean
  scale: number
  visible: boolean
  no_move: boolean
  topmost: boolean
}

export interface PetConfig {
  rx: number | null
  ry: number | null
  facing_right: boolean
  scale: number
  always_on_top: boolean
  no_move: boolean
  character: string | null
  /** PATCH 支持但 GET 不返回（运行时字段） */
  visible?: boolean
  // 行为引擎参数
  idle_ratio: number
  turn_ratio: number
  act_ratio: number
  act_interval_ms: number
  move_min_px: number
  move_max_px: number
  move_margin_px: number
  scale_steps: number[]
}

export interface PetInfo {
  id: string
  display_name: string
  source: string
  imported_at: number
  builtin: boolean
  video_count: number
  is_current: boolean
}

export interface ActionRow {
  action: string
  trigger: string
  weight: number
  enabled: boolean
}

export interface SystemInfo {
  version: string
  os: string
  port: number | null
  url: string | null
  config_dir: string
  yaml_path: string
  db_path: string
  assets_dir: string
  console_port: number | null
  log_level: string | null
}

/** 动画播放场合（与后端 trigger 对齐）。 */
export const TRIGGERS = [
  { id: 'idle', label: '待机时' },
  { id: 'turn', label: '转身时' },
  { id: 'move', label: '移动时' },
  { id: 'click', label: '点击它时' },
  { id: 'drag', label: '被拖拽时' },
  { id: 'idle_act', label: '其他时候' },
]

export function triggerLabel(id: string): string {
  return TRIGGERS.find((t) => t.id === id)?.label ?? id
}

async function req<T>(path: string, init?: RequestInit): Promise<ApiResp<T>> {
  const r = await fetch(path, init)
  return r
    .json()
    .catch(() => ({ ok: false, error: 'HTTP ' + r.status }))
}

function jsonInit(method: string, body: unknown): RequestInit {
  return {
    method,
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(body),
  }
}

export const api = {
  state: () => req<{ pet: PetState | null }>('/api/state'),

  config: () => req<PetConfig>('/api/config'),

  patchConfig: (patch: Partial<PetConfig>) => req<{ applied: string[] }>('/api/config', jsonInit('PATCH', patch)),

  play: (action: string) => req<{ played: string }>('/api/pet/play', jsonInit('POST', { action })),

  move: (x: number, y: number) => req<{ move: { x: number; y: number } }>('/api/pet/move', jsonInit('POST', { x, y })),

  say: (text: string, duration_ms?: number) =>
    req<{ say: string }>('/api/pet/say', jsonInit('POST', { text, duration_ms })),

  quit: () => req<{ msg: string }>('/api/quit', { method: 'POST' }),

  importZip: (file: File) =>
    fetch('/api/import', {
      method: 'POST',
      headers: { 'Content-Type': 'application/zip' },
      body: file,
    }).then((r) => r.json().catch(() => ({ ok: false, error: 'HTTP ' + r.status }))),

  // ---- 阶段 2：桌宠管理 / 设置 / 系统 ----

  pets: () => req<PetInfo[]>('/api/pets'),

  switchPet: (id: string) => req<{ current: string }>(`/api/pets/${encodeURIComponent(id)}/switch`, { method: 'POST' }),

  deletePet: (id: string, deleteFiles = false) =>
    req<{ deleted: string }>(`/api/pets/${encodeURIComponent(id)}${deleteFiles ? '?delete_files=1' : ''}`, {
      method: 'DELETE',
    }),

  petActions: (id: string) => req<ActionRow[]>(`/api/pets/${encodeURIComponent(id)}/actions`),

  savePetActions: (id: string, actions: ActionRow[]) =>
    req<{ saved: number }>(`/api/pets/${encodeURIComponent(id)}/actions`, jsonInit('PUT', actions)),

  /** 动画 webm 文件 URL（前端 <video> 直接播放） */
  webmUrl: (id: string, action: string) =>
    `/api/pets/${encodeURIComponent(id)}/webm/${encodeURIComponent(action)}`,

  settings: () => req<PetConfig>('/api/settings'),

  patchSettings: (patch: Partial<PetConfig>) => req<{ applied: string[] }>('/api/settings', jsonInit('PATCH', patch)),

  system: () => req<SystemInfo>('/api/system'),
}
