import { useEffect, useState } from 'react'
import { api, type ActionRow, type PetInfo } from '../api'

export default function DashboardPage() {
  const [pet, setPet] = useState<PetInfo | null>(null)
  const [actions, setActions] = useState<ActionRow[]>([])
  const [showUrl, setShowUrl] = useState<string | null>(null)
  const [sayText, setSayText] = useState('')
  const [playAction, setPlayAction] = useState('')
  const [msg, setMsg] = useState<{ ok: boolean; text: string } | null>(null)

  // 加载当前桌宠：轮询保持状态同步
  useEffect(() => {
    let alive = true
    async function load() {
      const pets = await api.pets()
      if (!alive) return
      if (!pets.ok || !pets.data) return
      const cur = pets.data.find((p) => p.is_current) ?? null
      setPet(cur)
      if (cur) {
        const acts = await api.petActions(cur.id)
        if (!alive) return
        if (acts.ok && acts.data) {
          setActions(acts.data)
          // 展示形象：优先待机动画，否则第一个启用的动画
          const idle =
            acts.data.find((a) => a.trigger === 'idle' && a.enabled) ??
            acts.data.find((a) => a.enabled) ??
            acts.data[0]
          if (idle) setShowUrl(api.webmUrl(cur.id, idle.action))
        }
      } else {
        setShowUrl(null)
        setActions([])
      }
    }
    load()
    const t = setInterval(load, 3000)
    return () => {
      alive = false
      clearInterval(t)
    }
  }, [])

  async function say() {
    const text = sayText.trim()
    if (!text) return
    const r = await api.say(text)
    setMsg(r.ok ? { ok: true, text: '它听到了～' } : { ok: false, text: r.error ?? '失败了' })
    if (r.ok) setSayText('')
  }

  async function play() {
    if (!playAction) return
    const r = await api.play(playAction)
    setMsg(r.ok ? { ok: true, text: `正在播放「${r.data?.played ?? playAction}」` } : { ok: false, text: r.error ?? '失败了' })
  }

  if (!pet) {
    return (
      <>
        <div className="card">
          <h1 className="page-title">还没有桌宠</h1>
          <p className="muted">先去「我的桌宠」页添加一个素材包吧～</p>
        </div>
      </>
    )
  }

  return (
    <>
      <div className="card">
        <div className="pet-stage">
          {showUrl && <video src={showUrl} autoPlay loop muted playsInline />}
          <div className="name">{pet.display_name}</div>
          <div className="sub">{pet.video_count} 个动画</div>
        </div>
      </div>

      {msg && <div className={'msg ' + (msg.ok ? 'ok' : 'err')}>{msg.text}</div>}

      <div className="card">
        <h2 className="card-title">对它说点什么</h2>
        <div className="row">
          <input
            type="text"
            placeholder="输入想让它说的话…"
            value={sayText}
            onChange={(e) => setSayText(e.target.value)}
            onKeyDown={(e) => e.key === 'Enter' && say()}
          />
          <button className="btn primary" disabled={!sayText.trim()} onClick={say}>
            说
          </button>
        </div>
      </div>

      <div className="card">
        <h2 className="card-title">播放动画</h2>
        <div className="row">
          <select value={playAction} onChange={(e) => setPlayAction(e.target.value)} style={{ flex: 1 }}>
            <option value="">选择一个动画…</option>
            {actions.map((a) => (
              <option key={a.action} value={a.action}>
                {a.action}
              </option>
            ))}
          </select>
          <button className="btn primary" disabled={!playAction} onClick={play}>
            播放
          </button>
        </div>
      </div>
    </>
  )
}
