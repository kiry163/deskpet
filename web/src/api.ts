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
  assets_dir: string | null
  character: string | null
  /** PATCH 支持但 GET 不返回（运行时字段） */
  visible?: boolean
}

async function req<T>(path: string, init?: RequestInit): Promise<ApiResp<T>> {
  const r = await fetch(path, init)
  return r
    .json()
    .catch(() => ({ ok: false, error: 'HTTP ' + r.status }))
}

export const api = {
  state: () => req<{ pet: PetState | null }>('/api/state'),

  config: () => req<PetConfig>('/api/config'),

  patchConfig: (patch: Partial<PetConfig>) =>
    req<{ applied: string[] }>('/api/config', {
      method: 'PATCH',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(patch),
    }),

  play: (action: string) =>
    req<{ played: string }>('/api/pet/play', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ action }),
    }),

  move: (x: number, y: number) =>
    req<{ move: { x: number; y: number } }>('/api/pet/move', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ x, y }),
    }),

  say: (text: string, duration_ms?: number) =>
    req<{ say: string }>('/api/pet/say', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ text, duration_ms }),
    }),

  quit: () => req<{ msg: string }>('/api/quit', { method: 'POST' }),

  importZip: (file: File) =>
    fetch('/api/import', {
      method: 'POST',
      headers: { 'Content-Type': 'application/zip' },
      body: file,
    }).then((r) => r.json().catch(() => ({ ok: false, error: 'HTTP ' + r.status }))),
}
