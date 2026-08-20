import { useCallback, useEffect, useState } from 'react'
import { api, type PetConfig } from '../api'

const SCALE_STEPS = [
  { v: 0.5, label: '50%' },
  { v: 0.72, label: '72%' },
  { v: 0.85, label: '85%' },
  { v: 1.0, label: '100%' },
]

export default function ConfigPage() {
  const [cfg, setCfg] = useState<PetConfig | null>(null)
  const [raw, setRaw] = useState('')
  const [msg, setMsg] = useState<{ ok: boolean; text: string } | null>(null)
  const [loadErr, setLoadErr] = useState('')

  const load = useCallback(async () => {
    try {
      const r = await api.config()
      if (r.ok && r.data) {
        setCfg(r.data)
        setRaw(JSON.stringify(r.data, null, 2))
        setLoadErr('')
      } else {
        setLoadErr(r.error ?? '读取配置失败')
      }
    } catch (e) {
      setLoadErr(`请求失败: ${e}`)
    }
  }, [])

  useEffect(() => {
    load()
  }, [load])

  const flash = (ok: boolean, text: string) => {
    setMsg({ ok, text })
    setTimeout(() => setMsg(null), 2500)
  }

  const patch = async (p: Partial<PetConfig>, label: string) => {
    try {
      const r = await api.patchConfig(p)
      if (r.ok) {
        flash(true, `${label} 已生效并保存到 config.json`)
        load()
      } else {
        flash(false, `${label} 失败: ${r.error ?? '未知错误'}`)
      }
    } catch (e) {
      flash(false, `${label} 请求失败: ${e}`)
    }
  }

  if (loadErr) {
    return (
      <div className="card">
        <h2>配置</h2>
        <div className="msg err">{loadErr}</div>
        <button className="btn" onClick={load} style={{ marginTop: 10 }}>
          重试
        </button>
      </div>
    )
  }

  if (!cfg) return <div className="card"><p className="muted">加载中…</p></div>

  return (
    <>
      {msg && <div className={'msg ' + (msg.ok ? 'ok' : 'err')}>{msg.text}</div>}

      <div className="card">
        <h2>
          外观
          <span className="hint">改动即时生效（热生效 + 落盘 config.json）</span>
        </h2>
        <div className="row">
          <label style={{ width: 64 }}>缩放</label>
          <select
            value={cfg.scale}
            onChange={(e) => patch({ scale: Number(e.target.value) }, '缩放')}
          >
            {SCALE_STEPS.map((s) => (
              <option key={s.v} value={s.v}>
                {s.label}（{s.v}）
              </option>
            ))}
          </select>
          <span className="muted">渲染分辨率四档</span>
        </div>
        <div className="check">
          <input
            type="checkbox"
            id="cfg-topmost"
            checked={cfg.always_on_top}
            onChange={(e) => patch({ always_on_top: e.target.checked }, '置顶')}
          />
          <label htmlFor="cfg-topmost">总是置顶（always_on_top）</label>
        </div>
        <div className="check">
          <input
            type="checkbox"
            id="cfg-facing"
            checked={cfg.facing_right}
            onChange={(e) => patch({ facing_right: e.target.checked }, '朝向')}
          />
          <label htmlFor="cfg-facing">默认朝右（facing_right）</label>
        </div>
        <div className="divider" />
        <div className="check">
          <input
            type="checkbox"
            id="cfg-nomove"
            checked={cfg.no_move}
            onChange={(e) => patch({ no_move: e.target.checked }, '不移动')}
          />
          <label htmlFor="cfg-nomove">锁定位置不自主移动（no_move）</label>
        </div>
        <div className="check">
          <input
            type="checkbox"
            id="cfg-visible"
            checked={cfg.visible ?? true}
            onChange={(e) => patch({ visible: e.target.checked }, '可见性')}
          />
          <label htmlFor="cfg-visible">显示桌宠（visible）</label>
        </div>
      </div>

      <div className="card">
        <h2>
          角色与素材
          <span className="hint">角色切换与素材根配置（M3 / 手改 config.json）</span>
        </h2>
        <dl className="kv">
          <dt>当前角色</dt>
          <dd>{cfg.character ?? '自动检测'}</dd>
          <dt>素材根</dt>
          <dd>{cfg.assets_dir ?? '自动解析（配置目录 assets/ → exe 旁 → 当前目录）'}</dd>
          <dt>位置 rx / ry</dt>
          <dd>
            {cfg.rx ?? '默认'} / {cfg.ry ?? '默认'}
          </dd>
        </dl>
        <p className="muted" style={{ marginBottom: 0 }}>
          导入素材包后当前角色会自动切换；行为策略（动作概率 / 移动参数 / 模式预设）与
          动作管理在 M2 提供表单。
        </p>
      </div>

      <div className="card">
        <h2>
          原始配置（config.json）
          <span className="badge">只读</span>
        </h2>
        <pre className="raw">{raw}</pre>
      </div>
    </>
  )
}
