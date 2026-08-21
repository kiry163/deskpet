import { useCallback, useEffect, useRef, useState } from 'react'
import { api, TRIGGERS, type ActionRow, type PetInfo } from '../api'

/** 桌宠卡片：展示其形象动画（待机或第一个动画）。 */
function PetCard({ pet, onSwitch, onDelete, onOpen }: {
  pet: PetInfo
  onSwitch: (id: string) => void
  onDelete: (p: PetInfo) => void
  onOpen: (id: string) => void
}) {
  const [url, setUrl] = useState<string | null>(null)
  useEffect(() => {
    let alive = true
    api.petActions(pet.id).then((r) => {
      if (!alive || !r.ok || !r.data) return
      const idle = r.data.find((a) => a.trigger === 'idle' && a.enabled) ?? r.data.find((a) => a.enabled) ?? r.data[0]
      if (idle) setUrl(api.webmUrl(pet.id, idle.action))
    })
    return () => {
      alive = false
    }
  }, [pet.id])

  return (
    <div className={'pet-card' + (pet.is_current ? ' current' : '')} onClick={() => onOpen(pet.id)}>
      {url ? <video src={url} autoPlay loop muted playsInline /> : <div className="muted" style={{ height: 100, display: 'flex', alignItems: 'center', justifyContent: 'center' }}>加载中…</div>}
      <h3>
        <span>{pet.display_name}</span>
        {pet.is_current && <span className="pill on">当前</span>}
      </h3>
      <div className="meta">{pet.video_count} 个动画</div>
      <div className="btns" onClick={(e) => e.stopPropagation()}>
        {!pet.is_current && (
          <button className="btn small primary" onClick={() => onSwitch(pet.id)}>
            设为当前
          </button>
        )}
        <button className="btn small danger" onClick={() => onDelete(pet)}>
          删除
        </button>
      </div>
    </div>
  )
}

export default function PetsPage() {
  const [pets, setPets] = useState<PetInfo[] | null>(null)
  const [selected, setSelected] = useState<PetInfo | null>(null)
  const [actions, setActions] = useState<ActionRow[]>([])
  const [dirty, setDirty] = useState(false)
  const [msg, setMsg] = useState<{ ok: boolean; text: string } | null>(null)

  // 导入
  const [drag, setDrag] = useState(false)
  const [busy, setBusy] = useState(false)
  const fileRef = useRef<HTMLInputElement>(null)

  const flash = (ok: boolean, text: string) => {
    setMsg({ ok, text })
    setTimeout(() => setMsg(null), 3000)
  }

  const loadPets = useCallback(async () => {
    const r = await api.pets()
    if (r.ok && r.data) {
      setPets(r.data)
      setSelected((sel) => (sel ? r.data!.find((p) => p.id === sel.id) ?? null : sel))
    }
  }, [])

  useEffect(() => {
    loadPets()
  }, [loadPets])

  // 打开桌宠详情 → 加载动作配置
  const openPet = useCallback(async (id: string) => {
    const r = await api.petActions(id)
    if (r.ok && r.data) setActions(r.data)
    setSelected(pets?.find((p) => p.id === id) ?? null)
    setDirty(false)
  }, [pets])

  useEffect(() => {
    if (selected && actions.length === 0) openPet(selected.id)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selected])

  async function upload(f: File) {
    if (!f.name.toLowerCase().endsWith('.zip')) {
      flash(false, '请选择 .zip 素材包')
      return
    }
    setBusy(true)
    try {
      const r = (await api.importZip(f)) as { ok: boolean; data?: { videos: number; id: string }; error?: string }
      if (r.ok && r.data) {
        flash(true, `添加成功：${r.data.videos} 个动画`)
        await loadPets()
      } else {
        flash(false, r.error ?? '添加失败')
      }
    } catch (e) {
      flash(false, `出错了：${e}`)
    }
    setBusy(false)
  }

  async function doSwitch(id: string) {
    const r = await api.switchPet(id)
    if (r.ok) {
      flash(true, '已切换，现在桌面上的就是它啦')
      await loadPets()
    } else {
      flash(false, r.error ?? '切换失败')
    }
  }

  async function doDelete(p: PetInfo) {
    if (!confirm(`确定删除「${p.display_name}」？\n\n只会把它从列表移除，素材文件会保留。`)) return
    const r = await api.deletePet(p.id)
    if (r.ok) {
      flash(true, '已删除')
      if (selected?.id === p.id) {
        setSelected(null)
        setActions([])
      }
      await loadPets()
    } else {
      flash(false, r.error ?? '删除失败')
    }
  }

  function updateAction(idx: number, patch: Partial<ActionRow>) {
    setActions((as) => as.map((a, i) => (i === idx ? { ...a, ...patch } : a)))
    setDirty(true)
  }

  async function saveActions() {
    if (!selected) return
    const r = await api.savePetActions(selected.id, actions)
    if (r.ok) {
      flash(true, '已保存')
      setDirty(false)
      await loadPets()
    } else {
      flash(false, r.error ?? '保存失败')
    }
  }

  // 按场景分组（禁用动画归入「暂不播放」）
  const groups = TRIGGERS.map((t) => ({
    ...t,
    items: actions.filter((a) => a.enabled && a.trigger === t.id),
  }))
  const disabled = actions.filter((a) => !a.enabled)

  if (selected) {
    return (
      <>
        {msg && <div className={'msg ' + (msg.ok ? 'ok' : 'err')}>{msg.text}</div>}
        <button className="btn small" onClick={() => setSelected(null)} style={{ marginBottom: 14 }}>
          ← 返回
        </button>
        <div className="card">
          <h2 className="card-title">
            {selected.display_name} 的动画
            <span className="muted" style={{ fontWeight: 400 }}>共 {actions.length} 个</span>
          </h2>

          {groups.map((g) => (
            <div className="act-group" key={g.id}>
              <div className="group-head">
                <span>{g.label}</span>
                <span className="count">{g.items.length} 个</span>
              </div>
              {g.items.length === 0 && (
                <div className="act-row muted" style={{ fontStyle: 'italic' }}>
                  还没有动画，可把下方动画的「什么时候播放」选到这里
                </div>
              )}
              {g.items.map((a, i) => {
                const idx = actions.indexOf(a)
                return (
                  <div className="act-row" key={a.action}>
                    <span className="name" title={a.action}>{a.action}</span>
                    <select
                      className="occ"
                      value={a.trigger}
                      onChange={(e) => updateAction(idx, { trigger: e.target.value })}
                    >
                      {TRIGGERS.map((t) => (
                        <option key={t.id} value={t.id}>{t.label}</option>
                      ))}
                    </select>
                    <div className="freq">
                      <input
                        type="range"
                        min={0}
                        max={10}
                        step={1}
                        value={a.weight}
                        onChange={(e) => {
                          const w = Number(e.target.value)
                          updateAction(idx, { weight: w, enabled: w > 0 })
                        }}
                      />
                      <span style={{ width: 52 }}>{a.weight > 0 ? `${a.weight} 级` : '不播放'}</span>
                    </div>
                  </div>
                )
              })}
            </div>
          ))}

          {disabled.length > 0 && (
            <div className="act-group">
              <div className="group-head">
                <span>暂不播放</span>
                <span className="count">{disabled.length} 个</span>
              </div>
              {disabled.map((a) => {
                const idx = actions.indexOf(a)
                return (
                  <div className="act-row off" key={a.action}>
                    <span className="name" title={a.action}>{a.action}</span>
                    <select
                      className="occ"
                      value={a.trigger}
                      onChange={(e) => updateAction(idx, { trigger: e.target.value })}
                    >
                      {TRIGGERS.map((t) => (
                        <option key={t.id} value={t.id}>{t.label}</option>
                      ))}
                    </select>
                    <div className="freq">
                      <input
                        type="range"
                        min={0}
                        max={10}
                        step={1}
                        value={0}
                        onChange={(e) => {
                          const w = Number(e.target.value)
                          updateAction(idx, { weight: w, enabled: w > 0 })
                        }}
                      />
                      <span style={{ width: 52 }}>不播放</span>
                    </div>
                  </div>
                )
              })}
            </div>
          )}

          <div className="row" style={{ marginTop: 16, marginBottom: 0 }}>
            <button className="btn primary" disabled={!dirty} onClick={saveActions}>
              保存{dirty ? '（有修改）' : ''}
            </button>
            {!dirty && <span className="muted">改一改上面的选项再保存</span>}
          </div>
        </div>
      </>
    )
  }

  return (
    <>
      {msg && <div className={'msg ' + (msg.ok ? 'ok' : 'err')}>{msg.text}</div>}

      <h1 className="page-title">我的桌宠</h1>

      <div className="card">
        <div className="pet-grid">
          {pets?.map((p) => (
            <PetCard key={p.id} pet={p} onSwitch={doSwitch} onDelete={doDelete} onOpen={openPet} />
          ))}
          <div
            className={'add-pet' + (drag ? ' drag' : '') + (busy ? ' busy' : '')}
            onClick={() => !busy && fileRef.current?.click()}
            onDragOver={(e) => {
              e.preventDefault()
              setDrag(true)
            }}
            onDragLeave={() => setDrag(false)}
            onDrop={(e) => {
              e.preventDefault()
              setDrag(false)
              if (!busy && e.dataTransfer.files[0]) upload(e.dataTransfer.files[0])
            }}
          >
            <span className="plus">＋</span>
            <span>{busy ? '正在添加…' : '添加新桌宠'}</span>
            <span className="muted" style={{ fontSize: 12 }}>选择一个 zip 素材包</span>
          </div>
        </div>
        <input
          ref={fileRef}
          type="file"
          accept=".zip,application/zip"
          style={{ display: 'none' }}
          onChange={(e) => {
            if (e.target.files?.[0]) upload(e.target.files[0])
            e.target.value = ''
          }}
        />
        <p className="muted" style={{ marginTop: 16, marginBottom: 0 }}>
          点击桌宠卡片，可以为它的每个动画安排「什么时候播放」和「出现频率」。
        </p>
      </div>
    </>
  )
}
