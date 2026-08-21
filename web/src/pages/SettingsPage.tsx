import { useCallback, useEffect, useState } from 'react'
import { api, type PetConfig } from '../api'

const SCALES = [
  { v: 0.5, label: '小' },
  { v: 0.72, label: '中' },
  { v: 0.85, label: '大' },
  { v: 1.0, label: '特大' },
]

export default function SettingsPage() {
  const [cfg, setCfg] = useState<PetConfig | null>(null)
  const [msg, setMsg] = useState<{ ok: boolean; text: string } | null>(null)

  const load = useCallback(async () => {
    const r = await api.settings()
    if (r.ok && r.data) setCfg(r.data)
  }, [])

  useEffect(() => {
    load()
  }, [load])

  const flash = (ok: boolean, text: string) => {
    setMsg({ ok, text })
    setTimeout(() => setMsg(null), 2500)
  }

  async function patch(p: Partial<PetConfig>, label: string) {
    const r = await api.patchSettings(p)
    if (r.ok) {
      flash(true, `${label}已保存`)
      load()
    } else {
      flash(false, r.error ?? '保存失败')
    }
  }

  if (!cfg) return <div className="card"><p className="muted">加载中…</p></div>

  return (
    <>
      {msg && <div className={'msg ' + (msg.ok ? 'ok' : 'err')}>{msg.text}</div>}

      <h1 className="page-title">设置</h1>

      <div className="card">
        <h2 className="card-title">大小</h2>
        <div className="scale-group">
          {SCALES.map((s) => (
            <button
              key={s.v}
              className={'scale-btn' + (cfg.scale === s.v ? ' active' : '')}
              onClick={() => patch({ scale: s.v }, '大小')}
            >
              {s.label}
            </button>
          ))}
        </div>
      </div>

      <div className="card">
        <h2 className="card-title">行为</h2>
        <div className="set-row">
          <div>
            <div className="label">总是在最前面</div>
            <div className="desc">不会被其他窗口挡住</div>
          </div>
          <label className="switch">
            <input
              type="checkbox"
              checked={cfg.always_on_top}
              onChange={(e) => patch({ always_on_top: e.target.checked }, '置顶')}
            />
            <span className="track" />
          </label>
        </div>
        <div className="set-row">
          <div>
            <div className="label">允许它自己走动</div>
            <div className="desc">关闭后它会在原地待着</div>
          </div>
          <label className="switch">
            <input
              type="checkbox"
              checked={!cfg.no_move}
              onChange={(e) => patch({ no_move: !e.target.checked }, '走动')}
            />
            <span className="track" />
          </label>
        </div>
        <div className="set-row">
          <div>
            <div className="label">默认朝右</div>
            <div className="desc">关闭后它默认脸朝左</div>
          </div>
          <label className="switch">
            <input
              type="checkbox"
              checked={cfg.facing_right}
              onChange={(e) => patch({ facing_right: e.target.checked }, '朝向')}
            />
            <span className="track" />
          </label>
        </div>
        <div className="set-row">
          <div>
            <div className="label">暂时隐藏</div>
            <div className="desc">不显示在桌面上（托盘图标里还能再打开）</div>
          </div>
          <label className="switch">
            <input
              type="checkbox"
              checked={cfg.visible ?? true}
              onChange={(e) => patch({ visible: e.target.checked }, '隐藏')}
            />
            <span className="track" />
          </label>
        </div>
      </div>
    </>
  )
}
