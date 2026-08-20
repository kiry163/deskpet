import { useCallback, useEffect, useRef, useState } from 'react'
import { api, type ApiResp, type PetState } from '../api'

interface LogEntry {
  t: string
  msg: string
  ok: boolean
}

const QUICK_ACTIONS = [
  { name: 'idle', label: '待机' },
  { name: 'turn', label: '转身' },
  { name: 'move', label: '移动' },
  { name: 'act', label: '表演' },
  { name: 'click', label: '点击' },
  { name: 'drag', label: '拖拽' },
]

const MOVE_PRESETS = [
  { label: '右下', x: 0.8, y: 0.9 },
  { label: '左下', x: 0.15, y: 0.9 },
  { label: '中央', x: 0.5, y: 0.5 },
  { label: '右上', x: 0.8, y: 0.15 },
]

export default function StatePage() {
  const [pet, setPet] = useState<PetState | null>(null)
  const [offline, setOffline] = useState(false)
  const [logs, setLogs] = useState<LogEntry[]>([])
  const [sayText, setSayText] = useState('')
  const [sayMs, setSayMs] = useState('4000')
  const [playAction, setPlayAction] = useState('')
  const [moveX, setMoveX] = useState(0.8)
  const [moveY, setMoveY] = useState(0.9)
  const logRef = useRef<HTMLPreElement>(null)

  const push = useCallback((msg: string, ok: boolean) => {
    setLogs((ls) => [{ t: new Date().toLocaleTimeString(), msg, ok }, ...ls].slice(0, 40))
  }, [])

  const refresh = useCallback(async () => {
    try {
      const r = await api.state()
      setOffline(false)
      if (r.ok && r.data) setPet(r.data.pet)
    } catch {
      setOffline(true)
    }
  }, [])

  useEffect(() => {
    refresh()
    const t = setInterval(refresh, 2000)
    return () => clearInterval(t)
  }, [refresh])

  useEffect(() => {
    if (logRef.current) logRef.current.scrollTop = 0
  }, [logs])

  const run = useCallback(
    async (label: string, p: Promise<ApiResp>) => {
      try {
        const r = await p
        push(`${label} → ${r.ok ? JSON.stringify(r.data ?? {}) : r.error ?? '未知错误'}`, r.ok)
      } catch (e) {
        push(`${label} → 请求失败: ${e}`, false)
      }
      refresh()
    },
    [push, refresh],
  )

  return (
    <>
      <div className="card">
        <h2>
          当前状态
          <span className="hint">每 2 秒自动刷新{offline && '（连接失败）'}</span>
        </h2>
        {pet === null ? (
          <p className="muted">桌宠未创建（无素材）。请先到「导入」页导入素材包。</p>
        ) : (
          <dl className="kv">
            <dt>当前动作</dt>
            <dd>
              {pet.anim ?? '—'}
              <span className="badge" style={{ marginLeft: 8 }}>
                scale {pet.scale}
              </span>
            </dd>
            <dt>位置</dt>
            <dd>
              ({pet.x}, {pet.y}) · 窗口 {pet.w}×{pet.h}
            </dd>
            <dt>朝向</dt>
            <dd>
              <span className={'pill ' + (pet.facing_right ? 'on' : 'off')}>
                {pet.facing_right ? '朝右' : '朝左'}
              </span>
            </dd>
            <dt>可见</dt>
            <dd>
              <span className={'pill ' + (pet.visible ? 'on' : 'off')}>
                {pet.visible ? '显示' : '隐藏'}
              </span>
            </dd>
            <dt>置顶</dt>
            <dd>
              <span className={'pill ' + (pet.topmost ? 'on' : 'off')}>
                {pet.topmost ? '是' : '否'}
              </span>
            </dd>
            <dt>不移动</dt>
            <dd>
              <span className={'pill ' + (pet.no_move ? 'on' : 'off')}>
                {pet.no_move ? '锁定' : '自由'}
              </span>
            </dd>
          </dl>
        )}
      </div>

      <div className="grid2">
        <div className="card">
          <h2>说话（气泡）</h2>
          <div className="row">
            <input
              type="text"
              placeholder="要说的内容…"
              value={sayText}
              onChange={(e) => setSayText(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === 'Enter' && sayText.trim())
                  run('say', api.say(sayText.trim(), parseInt(sayMs, 10) || undefined))
              }}
            />
          </div>
          <div className="row">
            <label>时长 ms</label>
            <input
              type="number"
              value={sayMs}
              min={500}
              step={500}
              style={{ width: 110 }}
              onChange={(e) => setSayMs(e.target.value)}
            />
            <button
              className="btn primary"
              disabled={!sayText.trim()}
              onClick={() => run('say', api.say(sayText.trim(), parseInt(sayMs, 10) || undefined))}
            >
              说
            </button>
          </div>
        </div>

        <div className="card">
          <h2>播放动作</h2>
          <div className="row">
            <input
              type="text"
              placeholder="动作名（精确 / 语义 / 模糊）"
              value={playAction}
              onChange={(e) => setPlayAction(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === 'Enter' && playAction.trim())
                  run('play', api.play(playAction.trim()))
              }}
            />
            <button
              className="btn primary"
              disabled={!playAction.trim()}
              onClick={() => run('play', api.play(playAction.trim()))}
            >
              播放
            </button>
          </div>
          <div className="row">
            {QUICK_ACTIONS.map((a) => (
              <button key={a.name} className="btn small" onClick={() => run('play', api.play(a.name))}>
                {a.label}
              </button>
            ))}
          </div>
        </div>
      </div>

      <div className="card">
        <h2>
          移动
          <span className="badge">归一化坐标 0..1</span>
        </h2>
        <div className="row">
          <label>X</label>
          <input
            type="range"
            min={0}
            max={1}
            step={0.01}
            value={moveX}
            onChange={(e) => setMoveX(Number(e.target.value))}
          />
          <span className="muted" style={{ width: 46, textAlign: 'right' }}>
            {moveX.toFixed(2)}
          </span>
        </div>
        <div className="row">
          <label>Y</label>
          <input
            type="range"
            min={0}
            max={1}
            step={0.01}
            value={moveY}
            onChange={(e) => setMoveY(Number(e.target.value))}
          />
          <span className="muted" style={{ width: 46, textAlign: 'right' }}>
            {moveY.toFixed(2)}
          </span>
        </div>
        <div className="row">
          <button className="btn primary" onClick={() => run('move', api.move(moveX, moveY))}>
            移动到这里
          </button>
          {MOVE_PRESETS.map((p) => (
            <button
              key={p.label}
              className="btn small"
              onClick={() => {
                setMoveX(p.x)
                setMoveY(p.y)
                run('move', api.move(p.x, p.y))
              }}
            >
              {p.label}
            </button>
          ))}
        </div>
      </div>

      <div className="card">
        <h2>指令日志</h2>
        <pre className="log" ref={logRef}>
          {logs.length === 0 ? '（暂无指令）' : ''}
          {logs.map((l, i) => (
            <div key={i} className={l.ok ? 'ok' : 'err'}>
              {l.t} {l.msg}
            </div>
          ))}
        </pre>
      </div>

      <div className="card">
        <h2>危险操作</h2>
        <button
          className="btn danger"
          onClick={() => {
            if (confirm('确定退出桌宠？')) run('quit', api.quit())
          }}
        >
          退出桌宠
        </button>
        <span className="muted" style={{ marginLeft: 12 }}>
          进程退出后需重新启动桌宠
        </span>
      </div>
    </>
  )
}
