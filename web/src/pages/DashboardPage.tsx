import { useEffect, useState } from 'react'
import { api, PetInfo, PetState, ActionItem, SystemInfo } from '../api'

export default function DashboardPage() {
  const [pets, setPets] = useState<PetInfo[]>([])
  const [state, setState] = useState<PetState | null>(null)
  const [actions, setActions] = useState<ActionItem[]>([])
  const [sys, setSys] = useState<SystemInfo | null>(null)
  const [selAction, setSelAction] = useState('')
  const [msg, setMsg] = useState<{ ok: boolean; text: string } | null>(null)

  const current = pets.find((p) => p.is_current) ?? pets[0]

  async function load() {
    const [p, s, sy] = await Promise.all([api.pets(), api.state(), api.system()])
    if (p.ok && p.data) setPets(p.data)
    if (s.ok && s.data) setState(s.data.pet ?? null)
    if (sy.ok && sy.data) setSys(sy.data)
  }

  useEffect(() => {
    load()
  }, [])

  useEffect(() => {
    if (current) {
      api.petActions(current.id).then((r) => {
        if (r.ok && r.data) setActions(r.data)
      })
    }
  }, [current?.id])

  const activeActions = actions.filter((a) => a.enabled).map((a) => ({
    label: a.display_name,
    value: a.action,
  }))

  async function act(fn: () => Promise<unknown>, okText: string) {
    const r = (await fn()) as { ok: boolean; error?: string }
    setMsg({ ok: r.ok, text: r.ok ? okText : r.error ?? '操作失败' })
    setTimeout(() => setMsg(null), 3000)
  }

  return (
    <div>
      <div className="card">
        <div className="current-pet">
          <PetImage pet={current} />
          <div className="cp-info">
            <div className="cp-name">
              {current ? current.display_name : '未导入桌宠'}
              {current?.is_current && <span className="badge green"><span className="dot" />当前</span>}
            </div>
            <div className="muted small">
              {current
                ? `全身照（来自待机动画）· ${current.video_count} 个动作`
                : '通过「宠物管理」导入素材包后即可运行'}
            </div>
            {current && (
              <div className="cp-badges">
                <span className="badge blue"><span className="dot" />状态:{stateLabel(state?.state_id)}</span>
                <span className="badge orange"><span className="dot" />当前动作:{state?.anim ?? '—'}</span>
              </div>
            )}
          </div>
        </div>
      </div>

      {msg && <div className={`msg ${msg.ok ? 'ok' : 'err'}`}>{msg.text}</div>}

      <div className="grid g3">
        <div className="card">
          <div className="label">位置</div>
          <div className="value">{state ? `(x:${state.x.toFixed(0)}, y:${state.y.toFixed(0)})` : '—'}</div>
        </div>
        <div className="card">
          <div className="label">大小 / 朝向</div>
          <div className="value">{state ? `${Math.round(state.scale * 100)}% · ${state.facing_right ? '朝右' : '朝左'}` : '—'}</div>
        </div>
        <div className="card">
          <div className="label">可见性</div>
          <div className="value">
            {state ? (
              <span className={`badge ${state.visible ? 'green' : ''}`}>
                <span className="dot" />{state.visible ? '显示中' : '已隐藏'}
              </span>
            ) : '—'}
          </div>
        </div>
      </div>

      <div className="card">
        <div className="section-h">快捷操作</div>
        <div className="quick">
          {current && (
            <>
              <select value={selAction} onChange={(e) => setSelAction(e.target.value)}>
                <option value="">选择要试播的动作…</option>
                {activeActions.map((a) => (
                  <option key={a.value} value={a.value}>{a.label}</option>
                ))}
              </select>
              <button className="btn primary" disabled={!selAction} onClick={() => act(() => api.play(selAction), '已播放')}>▶ 试播动作</button>
              <button className="btn" onClick={() => act(() => api.move(0.5, 0.62), '已移动')}>⇄ 移到屏幕下部</button>
              <button className="btn" onClick={() => act(() => api.patchConfig({ visible: !(state?.visible ?? true) }), state?.visible ? '已隐藏' : '已显示')}>
                {state?.visible ? '隐藏' : '显示'}
              </button>
            </>
          )}
          <button className="btn danger" onClick={() => act(() => api.quit(), '已关闭桌宠')}>关闭桌宠</button>
        </div>
      </div>

      <div className="card">
        <div className="section-h">系统信息</div>
        <div className="grid g3">
          <div><div className="label">版本</div><div className="value">{sys?.version ?? '—'}</div></div>
          <div><div className="label">端口</div><div className="value">{sys?.port ?? '—'}</div></div>
          <div><div className="label">日志级别</div><div className="value">{sys?.log_level ?? '—'}</div></div>
          <div><div className="label">数据库</div><div className="value small">{sys?.db_path ?? '—'}</div></div>
          <div><div className="label">素材目录</div><div className="value small">{sys?.assets_dir ?? '—'}</div></div>
          <div><div className="label">系统</div><div className="value">{sys?.os ?? '—'}</div></div>
        </div>
      </div>
    </div>
  )
}

function stateLabel(id: string | null | undefined): string {
  const map: Record<string, string> = { idle: '空闲', active: '活跃', lunch: '午休' }
  return (id && map[id]) || id || '—'
}

export function PetImage({ pet, className = '' }: { pet?: PetInfo; className?: string }) {
  const [err, setErr] = useState(false)
  useEffect(() => setErr(false), [pet?.id])
  // 始终尝试加载全身照；图片缺失/加载失败时才回退到首字占位
  if (!pet || err) {
    return <div className={`pet-ph ${className}`}>{pet?.display_name?.slice(0, 2) ?? '?'}</div>
  }
  return (
    <img
      className={`pet-ph img ${className}`}
      src={pet.fullbody_url}
      alt={pet.display_name}
      onError={() => setErr(true)}
    />
  )
}
