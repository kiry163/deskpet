// deskpet 控制台 API 封装：统一 {ok, data?, error?} 响应；导入走裸 zip body。

export interface ApiResp<T = unknown> {
  ok: boolean
  data?: T
  error?: string
}

export interface PetState {
  anim: string | null
  state_id: string | null
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

export interface TimeRule {
  start: string
  end: string
  enter: 'instant' | 'next_window'
  exit: 'at_end' | 'next_window'
}

export interface IntervalMs {
  min_ms: number
  max_ms: number
}

export interface StateDef {
  id: string
  name: string
  enabled: boolean
  weight: number
  time_rules: TimeRule[]
  interval: IntervalMs
}

export interface PetConfig {
  rx: number | null
  ry: number | null
  facing_right: boolean
  scale: number
  always_on_top: boolean
  no_move: boolean
  character: string | null
  behavior_states: StateDef[]
  move_min_px: number
  move_max_px: number
  move_margin_px: number
  scale_steps: number[]
  /** PATCH 支持但 GET 不返回（运行时字段） */
  visible?: boolean
}

export interface PetInfo {
  id: string
  display_name: string
  source: string
  imported_at: number
  builtin: boolean
  video_count: number
  is_current: boolean
  idle_action: string | null
  full_body_image: string | null
  fullbody_url: string
}

export interface ActionState {
  state_id: string
  weight: number
  enabled: boolean
}

export interface ActionItem {
  action: string
  display_name: string
  owner_kind: 'state' | 'interactive'
  kind: string | null // interactive → 'click' | 'drag'
  enabled: boolean
  states: ActionState[]
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

export interface ConvertJob {
  id: number
  src: string
  status: 'queued' | 'running' | 'done' | 'error'
  progress: number
  error: string | null
  created_at: number
}

export interface PetImportJob {
  id: number
  pet_id: string
  pet_name: string | null
  total: number
  done: number
  failed: number
  status: 'running' | 'done' | 'error'
  current_action: string | null
  error: string | null
  created_at: number
}

export interface PetVideoConvertPayload {
  name: string
  idle: string
  videos: { file: string; action: string }[]
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
    }).then((r) =>
      r
        .json()
        .catch(() => ({ ok: false, error: 'HTTP ' + r.status })),
    ),

  // ---- 桌宠管理 / 状态配置 / 设置 / 系统 ----

  pets: () => req<PetInfo[]>('/api/pets'),

  switchPet: (id: string) => req<{ current: string }>(`/api/pets/${encodeURIComponent(id)}/switch`, { method: 'POST' }),

  deletePet: (id: string, deleteFiles = false) =>
    req<{ deleted: string }>(`/api/pets/${encodeURIComponent(id)}${deleteFiles ? '?delete_files=1' : ''}`, {
      method: 'DELETE',
    }),

  updatePetName: (id: string, name: string) =>
    req<{ name: string }>(`/api/pets/${encodeURIComponent(id)}/name`, jsonInit('PUT', { name })),

  petActions: (id: string) => req<ActionItem[]>(`/api/pets/${encodeURIComponent(id)}/actions`),

  savePetActions: (id: string, actions: ActionItem[]) =>
    req<{ saved: number }>(`/api/pets/${encodeURIComponent(id)}/actions`, jsonInit('PUT', actions)),

  /** 动画 webm 文件 URL（前端 <video> 直接播放） */
  webmUrl: (id: string, action: string) =>
    `/api/pets/${encodeURIComponent(id)}/webm/${encodeURIComponent(action)}`,

  /** 全身照 URL（导入时自动从待机动画取帧） */
  fullbodyUrl: (id: string) => `/api/pets/${encodeURIComponent(id)}/fullbody`,

  /** 提交 mp4 绿幕转换作业（异步） */
  importVideo: (id: string, action: string, owner: string, file: File) =>
    fetch(
      `/api/pets/${encodeURIComponent(id)}/import/video?action=${encodeURIComponent(action)}&owner=${encodeURIComponent(owner)}`,
      { method: 'POST', headers: { 'Content-Type': 'video/mp4' }, body: file },
    ).then((r) => r.json().catch(() => ({ ok: false, error: 'HTTP ' + r.status }))),

  /** 该桌宠的转换作业列表与进度 */
  convertJobs: (id: string) => req<ConvertJob[]>(`/api/pets/${encodeURIComponent(id)}/jobs`),

  /** 视频包（仅源视频 zip）上传：校验 + 解压落位，返回 pet_id + 源视频名列表（不入库） */
  importPetVideo: (file: File) =>
    fetch('/api/import/pet-video', {
      method: 'POST',
      headers: { 'Content-Type': 'application/zip' },
      body: file,
    }).then((r) => r.json().catch(() => ({ ok: false, error: 'HTTP ' + r.status }))),

  /** 视频包 → 新建整只宠（异步批量建宠），返回 job_id */
  petVideoConvert: (petId: string, payload: PetVideoConvertPayload) =>
    req<{ job_id: number }>(
      `/api/import/pet-video/${encodeURIComponent(petId)}/convert`,
      jsonInit('POST', payload),
    ),

  /** 批量建宠作业进度 */
  petImportJob: (jobId: number) => req<PetImportJob>(`/api/import/jobs/${jobId}`),

  /** 一键导出宠物 zip（严格 §7.2 格式） */
  exportPetUrl: (id: string) => `/api/pets/${encodeURIComponent(id)}/export`,

  settings: () => req<PetConfig>('/api/settings'),

  patchSettings: (patch: Partial<PetConfig>) => req<{ applied: string[] }>('/api/settings', jsonInit('PATCH', patch)),

  system: () => req<SystemInfo>('/api/system'),
}
