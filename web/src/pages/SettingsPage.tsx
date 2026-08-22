import { useCallback, useEffect, useState } from 'react'
import { api, PetConfig, SystemInfo } from '../api'

export default function SettingsPage() {
  const [cfg, setCfg] = useState<PetConfig | null>(null)
  const [sys, setSys] = useState<SystemInfo | null>(null)
  const [msg, setMsg] = useState<{ ok: boolean; text: string } | null>(null)

  const load = useCallback(async () => {
    const [c, s] = await Promise.all([api.settings(), api.system()])
    if (c.ok && c.data) setCfg(c.data)
    if (s.ok && s.data) setSys(s.data)
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

  const steps = cfg.scale_steps.length ? cfg.scale_steps : [0.5, 0.72, 0.85, 1.0]

  return (
    <div>
      {msg && <div className={`msg ${msg.ok ? 'ok' : 'err'}`}>{msg.text}</div>}

      <div className="card">
        <div className="section-h">外观</div>
        <div className="set-row">
          <div>
            <div className="label">大小</div>
            <div className="desc">桌宠在屏幕上的缩放比例</div>
          </div>
          <div className="scale-group">
            {steps.map((s) => (
              <button key={s} className={`scale-btn${Math.abs(cfg.scale - s) < 0.02 ? ' active' : ''}`}
                onClick={() => patch({ scale: s }, '大小')}>{Math.round(s * 100)}%</button>
            ))}
          </div>
        </div>
        <div className="set-row">
          <div>
            <div className="label">朝向</div>
            <div className="desc">默认脸朝哪一侧</div>
          </div>
          <div className="seg">
            <button className={cfg.facing_right ? 'on' : ''} onClick={() => patch({ facing_right: true }, '朝向')}>朝右</button>
            <button className={!cfg.facing_right ? 'on' : ''} onClick={() => patch({ facing_right: false }, '朝向')}>朝左</button>
          </div>
        </div>
        <div className="set-row">
          <div>
            <div className="label">总在最前面</div>
            <div className="desc">不会被其他窗口挡住</div>
          </div>
          <Toggle on={cfg.always_on_top} onClick={() => patch({ always_on_top: !cfg.always_on_top }, '置顶')} />
        </div>
        <div className="set-row">
          <div>
            <div className="label">允许它自己走动</div>
            <div className="desc">关闭后它会在原地待着</div>
          </div>
          <Toggle on={!cfg.no_move} onClick={() => patch({ no_move: !cfg.no_move }, '走动')} />
        </div>
      </div>

      <div className="card">
        <div className="section-h">行为（移动）</div>
        <div className="muted small" style={{ marginBottom: 12 }}>移动距离与边界的微调；动作间隔在「状态配置」按状态设置</div>
        <div className="set-row">
          <div>
            <div className="label">最小移动距离</div>
            <div className="desc">自主动作每次移动的最短路径（像素）</div>
          </div>
          <input type="number" value={Math.round(cfg.move_min_px)} onChange={(e) => setCfg({ ...cfg, move_min_px: Number(e.target.value) })}
            onBlur={() => patch({ move_min_px: cfg.move_min_px }, '最小移动')} />
        </div>
        <div className="set-row">
          <div>
            <div className="label">最大移动距离</div>
            <div className="desc">自主动作每次移动的最长路径（像素）</div>
          </div>
          <input type="number" value={Math.round(cfg.move_max_px)} onChange={(e) => setCfg({ ...cfg, move_max_px: Number(e.target.value) })}
            onBlur={() => patch({ move_max_px: cfg.move_max_px }, '最大移动')} />
        </div>
        <div className="set-row">
          <div>
            <div className="label">边界留白</div>
            <div className="desc">移动时离屏幕边缘留出的间距（像素）</div>
          </div>
          <input type="number" value={Math.round(cfg.move_margin_px)} onChange={(e) => setCfg({ ...cfg, move_margin_px: Number(e.target.value) })}
            onBlur={() => patch({ move_margin_px: cfg.move_margin_px }, '边界留白')} />
        </div>
      </div>

      <div className="card">
        <div className="section-h">视频转换（默认参数）</div>
        <p className="hint">从「导入动作」上传 mp4 时用。大多数情况不用改。</p>
        <div className="grid g2">
          <div><div className="label">输出画布</div><div className="value">640 × 360</div></div>
          <div><div className="label">去绿边强度</div><div className="value">90</div></div>
        </div>
      </div>

      <div className="card">
        <div className="section-h">系统级（保存在配置文件）</div>
        <div className="grid g2">
          <div><div className="label">版本</div><div className="value">{sys?.version ?? '—'}</div></div>
          <div><div className="label">端口</div><div className="value">{sys?.port ?? '—'}</div></div>
          <div><div className="label">控制台端口（配置）</div><div className="value">{sys?.console_port ?? '—'}</div></div>
          <div><div className="label">日志级别</div><div className="value">{sys?.log_level ?? '—'}</div></div>
          <div><div className="label">数据库</div><div className="value small">{sys?.db_path ?? '—'}</div></div>
          <div><div className="label">素材目录</div><div className="value small">{sys?.assets_dir ?? '—'}</div></div>
          <div><div className="label">配置文件</div><div className="value small">{sys?.yaml_path ?? '—'}</div></div>
        </div>
      </div>
    </div>
  )
}

function Toggle({ on, onClick }: { on: boolean; onClick: () => void }) {
  return <span className={`switch${on ? ' on' : ''}`} onClick={onClick} />
}
