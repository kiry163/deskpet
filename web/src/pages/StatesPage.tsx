import { useCallback, useEffect, useState } from 'react'
import { api, StateDef, TimeRule } from '../api'

const BUILTIN = new Set(['idle', 'active', 'lunch'])

export default function StatesPage() {
  const [states, setStates] = useState<StateDef[] | null>(null)
  const [original, setOriginal] = useState<StateDef[]>([])
  const [msg, setMsg] = useState<{ ok: boolean; text: string } | null>(null)

  const load = useCallback(async () => {
    const r = await api.config()
    if (r.ok && r.data) {
      setStates(r.data.behavior_states)
      setOriginal(JSON.parse(JSON.stringify(r.data.behavior_states)))
    }
  }, [])

  useEffect(() => {
    load()
  }, [load])

  const flash = (ok: boolean, text: string) => {
    setMsg({ ok, text })
    setTimeout(() => setMsg(null), 3000)
  }

  function upate(idx: number, fn: (s: StateDef) => StateDef) {
    setStates((ss) => ss!.map((s, i) => (i === idx ? fn(s) : s)))
  }

  function addState() {
    const name = '新状态'
    setStates((ss) => [
      ...(ss ?? []),
      {
        id: 'state_' + Date.now(),
        name,
        enabled: true,
        weight: 0.2,
        time_rules: [],
        interval: { min_ms: 5000, max_ms: 8000 },
      },
    ])
  }

  function removeState(idx: number) {
    if (!confirm('确定删除这个状态？宠物动作绑定到它的归属会一并移除？')) return
    setStates((ss) => ss!.filter((_, i) => i !== idx))
  }

  async function save() {
    if (!states) return
    // 归一：interval 已是 ms；weight 限制 0..1
    const payload = states.map((s) => ({
      ...s,
      weight: Math.max(0, Math.min(1, s.weight)),
      interval: { min_ms: Math.max(0, s.interval.min_ms), max_ms: Math.max(s.interval.min_ms, s.interval.max_ms) },
    }))
    const r = await api.patchSettings({ behavior_states: payload })
    if (r.ok) {
      flash(true, '状态配置已保存')
      load()
    } else {
      flash(false, r.error ?? '保存失败')
    }
  }

  const dirty = !!states && JSON.stringify(states) !== JSON.stringify(original)

  if (!states) return <div className="card"><p className="muted">加载中…</p></div>

  return (
    <div>
      {msg && <div className={`msg ${msg.ok ? 'ok' : 'err'}`}>{msg.text}</div>}

      <div className="card block-head">
        <div>
          <div style={{ fontWeight: 600 }}>状态配置（程序级）</div>
          <div className="muted small">全局的空闲/活跃/午休；宠物动作绑定到这里的某个状态上，动作间隔按状态设置</div>
        </div>
        <button className="btn primary" onClick={addState}>＋ 新增状态</button>
      </div>

      <div className="grid g2">
        {states.map((s, i) => (
          <StateCard key={s.id} s={s} idx={i} builtin={BUILTIN.has(s.id)} update={upate} remove={removeState} />
        ))}
      </div>

      <div className="card">
        <div className="section-h">说明</div>
        <div className="muted">
          「出现频率」是该状态在自由时段的加权随机占比；设为 0 % 且设置固定时段后，只会在该时段出现。
          「动作间隔」决定进入该状态后，播完一个动作等多久再播下一个。
        </div>
      </div>

      <div className="card save-row">
        <button className="btn primary" disabled={!dirty} onClick={save}>保存{dirty ? '（有修改）' : ''}</button>
        {!dirty && <span className="muted small">改一改上面的频率、间隔或时段再保存</span>}
      </div>
    </div>
  )
}

function StateCard({ s, idx, builtin, update, remove }: {
  s: StateDef; idx: number; builtin: boolean; update: (i: number, fn: (x: StateDef) => StateDef) => void; remove: (i: number) => void
}) {
  const set = (fn: (x: StateDef) => StateDef) => update(idx, fn)
  return (
    <div className={`card state-card${!s.enabled ? ' off' : ''}`}>
      <div className="state-head">
        <input
          className="state-name"
          type="text"
          value={s.name}
          onChange={(e) => set((x) => ({ ...x, name: e.target.value }))}
        />
        {builtin ? <span className="badge blue">内置</span> : <button className="btn sm danger" onClick={() => remove(idx)}>删除</button>}
        <Toggle on={s.enabled} onClick={() => set((x) => ({ ...x, enabled: !x.enabled }))} />
      </div>

      <div className="label">出现频率</div>
      <input type="range" min={0} max={100} value={Math.round(s.weight * 100)}
        onChange={(e) => set((x) => ({ ...x, weight: Number(e.target.value) / 100 }))} />
      <div className="small muted">{Math.round(s.weight * 100)}%</div>

      <div className="label">动作间隔</div>
      <div className="time">
        <input type="number" min={0} value={s.interval.min_ms / 1000}
          onChange={(e) => set((x) => ({ ...x, interval: { ...x.interval, min_ms: Number(e.target.value) * 1000 } }))} />
        <span>~</span>
        <input type="number" min={0} value={s.interval.max_ms / 1000}
          onChange={(e) => set((x) => ({ ...x, interval: { ...x.interval, max_ms: Number(e.target.value) * 1000 } }))} />
        <span>秒</span>
      </div>

      <div className="label">指定时间段</div>
      {s.time_rules.length === 0 && <div className="muted small">未设置 → 按频率出现</div>}
      {s.time_rules.map((r, ri) => (
        <div key={ri} className="time-rule">
          <div className="time">
            <input type="time" value={r.start} onChange={(e) => setRule(s, ri, { start: e.target.value }, set)} />
            <span>到</span>
            <input type="time" value={r.end} onChange={(e) => setRule(s, ri, { end: e.target.value }, set)} />
          </div>
          <div className="time-rule-seg">
            <div className="seg">
              <button className={r.enter === 'instant' ? 'on' : ''} onClick={() => setRule(s, ri, { enter: 'instant' }, set)}>准点进入</button>
              <button className={r.enter === 'next_window' ? 'on' : ''} onClick={() => setRule(s, ri, { enter: 'next_window' }, set)}>顺延</button>
            </div>
            <div className="seg">
              <button className={r.exit === 'at_end' ? 'on' : ''} onClick={() => setRule(s, ri, { exit: 'at_end' }, set)}>到点结束</button>
              <button className={r.exit === 'next_window' ? 'on' : ''} onClick={() => setRule(s, ri, { exit: 'next_window' }, set)}>顺延</button>
            </div>
          </div>
          <button className="btn sm danger" onClick={() => set((x) => ({ ...x, time_rules: x.time_rules.filter((_, j) => j !== ri) }))}>移除</button>
        </div>
      ))}
      <button className="btn sm ghost" onClick={() => set((x) => ({ ...x, time_rules: [...x.time_rules, { start: '12:30', end: '14:00', enter: 'instant', exit: 'at_end' }] }))}>
        ＋ 添加时段
      </button>
    </div>
  )
}

function setRule(s: StateDef, ri: number, patch: Partial<TimeRule>, set: (fn: (x: StateDef) => StateDef) => void) {
  set((x) => ({
    ...x,
    time_rules: x.time_rules.map((r, j) => (j === ri ? { ...r, ...patch } : r)),
  }))
}

function Toggle({ on, onClick }: { on: boolean; onClick: () => void }) {
  return <span className={`switch${on ? ' on' : ''}`} onClick={onClick} />
}
