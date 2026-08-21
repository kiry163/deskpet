import { useCallback, useEffect, useState } from 'react'
import { api, type PetConfig } from '../api'

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

  const pct = (v: number) => Math.round(v * 100)

  return (
    <>
      {msg && <div className={'msg ' + (msg.ok ? 'ok' : 'err')}>{msg.text}</div>}

      <h1 className="page-title">设置</h1>

      <div className="card">
        <h2 className="card-title">大小</h2>
        <div className="scale-group">
          {(cfg.scale_steps.length ? cfg.scale_steps : [0.5, 0.72, 0.85, 1.0]).map((s) => (
            <button
              key={s}
              className={'scale-btn' + (Math.abs(cfg.scale - s) < 0.02 ? ' active' : '')}
              onClick={() => patch({ scale: s }, '大小')}
            >
              {Math.round(s * 100)}%
            </button>
          ))}
        </div>
      </div>

      <div className="card">
        <h2 className="card-title">行为</h2>
        <div className="set-row" style={{ flexDirection: 'column', alignItems: 'stretch' }}>
          <div>
            <div className="label">待机 {pct(cfg.idle_ratio)}%</div>
            <div className="desc">安静待着的时间占比（转向/动作/移动占其余）</div>
          </div>
          <input
            type="range"
            min={0}
            max={100}
            value={pct(cfg.idle_ratio)}
            onChange={(e) => patch({ idle_ratio: Number(e.target.value) / 100 }, '待机占比')}
          />
        </div>
        <div className="set-row" style={{ flexDirection: 'column', alignItems: 'stretch' }}>
          <div>
            <div className="label">转身 {pct(cfg.turn_ratio)}%</div>
            <div className="desc">东张西望、转身的频率（含待机）</div>
          </div>
          <input
            type="range"
            min={0}
            max={100}
            value={pct(cfg.turn_ratio)}
            onChange={(e) => patch({ turn_ratio: Number(e.target.value) / 100 }, '转身占比')}
          />
        </div>
        <div className="set-row" style={{ flexDirection: 'column', alignItems: 'stretch' }}>
          <div>
            <div className="label">闲时表演 {pct(cfg.act_ratio)}%</div>
            <div className="desc">做各种小动作的频率（含待机、转身；其余为走动）</div>
          </div>
          <input
            type="range"
            min={0}
            max={100}
            value={pct(cfg.act_ratio)}
            onChange={(e) => patch({ act_ratio: Number(e.target.value) / 100 }, '闲时表演占比')}
          />
        </div>
        <div className="set-row" style={{ flexDirection: 'column', alignItems: 'stretch' }}>
          <div>
            <div className="label">动作间隔 {cfg.act_interval_ms / 1000} 秒</div>
            <div className="desc">每个动作结束后停顿多久再继续（0 = 不停顿）</div>
          </div>
          <input
            type="range"
            min={0}
            max={10000}
            step={500}
            value={cfg.act_interval_ms}
            onChange={(e) => patch({ act_interval_ms: Number(e.target.value) }, '动作间隔')}
          />
        </div>
      </div>

      <div className="card">
        <h2 className="card-title">其他</h2>
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
