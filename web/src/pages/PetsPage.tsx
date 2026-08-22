import { useCallback, useEffect, useRef, useState, type RefObject } from 'react'
import { api, ActionItem, ConvertJob, PetImportJob, PetInfo, StateDef } from '../api'
import { PetImage } from './DashboardPage'

type Owner = 'state' | 'click' | 'drag'

interface Binding {
  weight: number
  enabled: boolean
}

interface EditAction {
  action: string
  display_name: string
  owner: Owner
  enabled: boolean
  bindings: Record<string, Binding>
}

const OWNER_LABEL: Record<Owner, string> = { state: '按时段', click: '点击', drag: '拖拽' }

function toEdit(a: ActionItem): EditAction {
  const owner: Owner = a.owner_kind === 'interactive' ? (a.kind === 'click' ? 'click' : 'drag') : 'state'
  const bindings: Record<string, Binding> = {}
  for (const st of a.states) bindings[st.state_id] = { weight: st.weight, enabled: st.enabled }
  return { action: a.action, display_name: a.display_name, owner, enabled: a.enabled, bindings }
}

export default function PetsPage() {
  const [pets, setPets] = useState<PetInfo[] | null>(null)
  const [selected, setSelected] = useState<PetInfo | null>(null)
  const [states, setStates] = useState<StateDef[]>([])
  const [actions, setActions] = useState<EditAction[]>([])
  const [view, setView] = useState<'grid' | 'list'>('grid')
  const [dirty, setDirty] = useState(false)
  const [msg, setMsg] = useState<{ ok: boolean; text: string } | null>(null)
  const [editingName, setEditingName] = useState('')

  const [drag, setDrag] = useState(false)
  const [busy, setBusy] = useState(false)
  const fileRef = useRef<HTMLInputElement>(null)

  // ---- 从视频新建宠物（§7.3 视频包 → 整只宠） ----
  const [videoWizard, setVideoWizard] = useState(false)
  const [vzBusy, setVzBusy] = useState(false)
  const [vzStaging, setVzStaging] = useState<{ pet_id: string; videos: string[] } | null>(null)
  const [vzName, setVzName] = useState('')
  const [vzActions, setVzActions] = useState<Record<string, string>>({})
  const [vzIdle, setVzIdle] = useState('')
  const [vzJobId, setVzJobId] = useState<number | null>(null)
  const [vzJob, setVzJob] = useState<PetImportJob | null>(null)
  const [vzDrag, setVzDrag] = useState(false)
  const vzFileRef = useRef<HTMLInputElement>(null)

  // ---- 从视频导入动作（mp4 异步转换作业） ----
  const [jobs, setJobs] = useState<ConvertJob[]>([])
  const [impFile, setImpFile] = useState<File | null>(null)
  const [impName, setImpName] = useState('')
  const [impOwner, setImpOwner] = useState<Owner>('state')
  const [impBusy, setImpBusy] = useState(false)
  const [impDrag, setImpDrag] = useState(false)
  const impFileRef = useRef<HTMLInputElement>(null)
  const targetJobRef = useRef<number | null>(null)

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

  // 重载当前宠物的动作列表（转换完成/导入后用）
  const reloadActions = useCallback(async (id: string) => {
    const r = await api.petActions(id)
    if (r.ok && r.data) setActions(r.data.map(toEdit))
  }, [])

  // 轮询该宠物的转换作业；被跟踪的作业结束（done/error）后重载动作
  useEffect(() => {
    if (!selected) return
    const sel = selected
    let alive = true
    async function tick() {
      const r = await api.convertJobs(sel.id)
      if (!alive || !r.ok || !r.data) return
      setJobs(r.data)
      const tj = targetJobRef.current
      if (tj != null) {
        const job = r.data.find((j) => j.id === tj)
        if (job && (job.status === 'done' || job.status === 'error')) {
          targetJobRef.current = null
          reloadActions(sel.id)
          loadPets()
        }
      }
    }
    tick()
    const t = setInterval(tick, 1200)
    return () => {
      alive = false
      clearInterval(t)
    }
  }, [selected?.id, reloadActions, loadPets])

  async function submitVideo() {
    if (!selected || !impFile) return
    const action = impName.trim()
    if (!action) {
      flash(false, '请填写动作名')
      return
    }
    setImpBusy(true)
    const r = (await api.importVideo(selected.id, action, impOwner, impFile)) as {
      ok: boolean
      data?: { job_id: number; action: string; owner: string }
      error?: string
    }
    setImpBusy(false)
    if (r.ok && r.data) {
      targetJobRef.current = r.data.job_id
      flash(true, `已提交转换作业 #${r.data.job_id}，完成会自动注册为动作「${r.data.action}」`)
      setImpFile(null)
      setImpName('')
    } else {
      flash(false, r.error ?? '提交失败')
    }
  }

  function pickImpFile(f: File) {
    setImpFile(f)
    if (!impName.trim()) {
      setImpName(f.name.replace(/\.(mp4|mov)$/i, ''))
    }
  }

  useEffect(() => {
    loadPets()
    api.config().then((r) => {
      if (r.ok && r.data) setStates(r.data.behavior_states)
    })
  }, [loadPets])

  const openPet = useCallback(
    async (id: string) => {
      const [acts] = await Promise.all([api.petActions(id)])
      if (acts.ok && acts.data) setActions(acts.data.map(toEdit))
      setSelected(pets?.find((p) => p.id === id) ?? null)
      setDirty(false)
      setEditingName('')
    },
    [pets],
  )

  async function upload(f: File) {
    if (!f.name.toLowerCase().endsWith('.zip')) {
      flash(false, '请选择 .zip 素材包')
      return
    }
    setBusy(true)
    const r = (await api.importZip(f)) as { ok: boolean; data?: { videos: number; id: string; display_name: string }; error?: string }
    if (r.ok && r.data) {
      flash(true, `导入成功：${r.data.display_name}，${r.data.videos} 个动作`)
      setSelected(null)
      await loadPets()
    } else {
      flash(false, r.error ?? '导入失败')
    }
    setBusy(false)
  }

  // ---- 从视频新建宠物 ----
  async function uploadVideoZip(f: File) {
    if (!f.name.toLowerCase().endsWith('.zip')) {
      flash(false, '请选择 .zip 视频包')
      return
    }
    setVzBusy(true)
    const r = (await api.importPetVideo(f)) as { ok: boolean; data?: { pet_id: string; videos: string[] }; error?: string }
    if (r.ok && r.data) {
      const d = r.data
      const acts: Record<string, string> = {}
      for (const v of d.videos) acts[v] = v
      setVzStaging({ pet_id: d.pet_id, videos: d.videos })
      setVzActions(acts)
      setVzIdle(d.videos[0] ?? '')
      setVzName('')
      setVzJob(null)
      setVzJobId(null)
      flash(true, `已解析 ${d.videos.length} 个源视频，请命名并指定待机`)
    } else {
      flash(false, r.error ?? '解析失败')
    }
    setVzBusy(false)
  }

  async function startVideoConvert() {
    if (!vzStaging) return
    const vids = Object.entries(vzActions)
      .map(([file, action]) => ({ file, action: action.trim() }))
      .filter((x) => x.action)
    if (vids.length === 0) {
      flash(false, '请至少给一个动作命名')
      return
    }
    if (!vzName.trim()) {
      flash(false, '请填宠物名')
      return
    }
    if (!vzIdle) {
      flash(false, '请指定待机动画')
      return
    }
    setVzBusy(true)
    const r = (await api.petVideoConvert(vzStaging.pet_id, { name: vzName.trim(), idle: vzIdle, videos: vids })) as {
      ok: boolean
      data?: { job_id: number }
      error?: string
    }
    setVzBusy(false)
    if (r.ok && r.data) {
      setVzJobId(r.data.job_id)
      setVzJob({
        id: r.data.job_id,
        pet_id: vzStaging.pet_id,
        pet_name: vzName,
        total: vids.length,
        done: 0,
        failed: 0,
        status: 'running',
        current_action: '',
        error: null,
        created_at: 0,
      })
      flash(true, '开始批量建宠…')
    } else {
      flash(false, r.error ?? '启动失败')
    }
  }

  // 轮询批量建宠作业
  useEffect(() => {
    if (!vzJobId) return
    const jid = vzJobId
    let alive = true
    async function tick() {
      const r = await api.petImportJob(jid)
      if (!alive) return
      if (r.ok && r.data) {
        setVzJob(r.data)
        if (r.data.status === 'done' || r.data.status === 'error') {
          setVzJobId(null)
          if (r.data.status === 'done') {
            flash(true, `建宠完成：${r.data.pet_id}`)
            await loadPets()
            setVideoWizard(false)
          } else {
            flash(false, r.data.error ?? '建宠失败')
          }
        }
      }
    }
    tick()
    const t = setInterval(tick, 1500)
    return () => {
      alive = false
      clearInterval(t)
    }
  }, [vzJobId, loadPets])

  function exportPet(id: string) {
    const a = document.createElement('a')
    a.href = api.exportPetUrl(id)
    a.download = ''
    document.body.appendChild(a)
    a.click()
    a.remove()
  }

  async function doSwitch(id: string) {
    const r = await api.switchPet(id)
    flash(r.ok, r.ok ? '已切换为当前桌宠' : r.error ?? '切换失败')
    await loadPets()
  }

  async function doDelete(p: PetInfo) {
    const withFiles = confirm(`确定删除「${p.display_name}」？`)
    if (!withFiles) return
    const r = await api.deletePet(p.id, true)
    if (r.ok) {
      flash(true, '已删除')
      if (selected?.id === p.id) setSelected(null)
      await loadPets()
    } else {
      flash(false, r.error ?? '删除失败')
    }
  }

  async function doRename() {
    if (!selected || !editingName.trim()) return
    const r = await api.updatePetName(selected.id, editingName.trim())
    if (r.ok) {
      flash(true, '名称已更新')
      setEditingName('')
      await loadPets()
    } else {
      flash(false, r.error ?? '更新失败')
    }
  }

  const patchAction = (idx: number, fn: (a: EditAction) => EditAction) => {
    setActions((as) => as.map((a, i) => (i === idx ? fn(a) : a)))
    setDirty(true)
  }

  async function saveActions() {
    if (!selected) return
    const payload: ActionItem[] = actions.map((a) => ({
      action: a.action,
      display_name: a.display_name,
      owner_kind: a.owner === 'state' ? 'state' : 'interactive',
      kind: a.owner === 'state' ? null : a.owner,
      enabled: a.enabled,
      states:
        a.owner === 'state'
          ? Object.entries(a.bindings)
              .filter(([, b]) => b.enabled)
              .map(([state_id, b]) => ({ state_id, weight: b.weight, enabled: true }))
          : [],
    }))
    const r = await api.savePetActions(selected.id, payload)
    if (r.ok) {
      flash(true, '动作配置已保存')
      setDirty(false)
      await loadPets()
    } else {
      flash(false, r.error ?? '保存失败')
    }
  }

  // ---------- 详情视图：宠物信息 + 动作管理 ----------
  if (selected) {
    return (
      <>
        {msg && <div className={`msg ${msg.ok ? 'ok' : 'err'}`}>{msg.text}</div>}
        <button className="btn ghost" onClick={() => setSelected(null)}>← 返回宠物列表</button>

        <div className="card">
          <div className="detail-head">
            <PetImage pet={selected} className="detail-ph" />
            <div className="detail-info">
              <div className="cp-name">{selected.display_name}</div>
              <div className="muted small">全身照自动取自待机动画（不可改）· 待机：{selected.idle_action ?? '—'}</div>
              <div className="detail-actions">
                <button className="btn sm" onClick={() => setEditingName(selected.display_name)}>✎ 编辑名称</button>
                {!selected.is_current && (
                  <button className="btn sm primary" onClick={() => doSwitch(selected.id)}>设为当前</button>
                )}
                <button className="btn sm" onClick={() => exportPet(selected.id)}>⬇ 导出 zip</button>
              </div>
              {editingName !== '' && (
                <div className="edit-name">
                  <input type="text" value={editingName} onChange={(e) => setEditingName(e.target.value)} />
                  <button className="btn primary sm" onClick={doRename}>保存</button>
                  <button className="btn sm" onClick={() => setEditingName('')}>取消</button>
                </div>
              )}
            </div>
          </div>
        </div>

        <div className="card">
          <div className="block-head">
            <div className="section-h" style={{ margin: 0 }}>动作管理</div>
            <div className="seg">
              <button className={view === 'grid' ? 'on' : ''} onClick={() => setView('grid')}>▦ 网格</button>
              <button className={view === 'list' ? 'on' : ''} onClick={() => setView('list')}>☰ 列表</button>
            </div>
          </div>
          <div className="muted small" style={{ marginBottom: 14 }}>
            动作可同时归入多个状态；每个状态的出现概率单独设置；点击/拖拽属于交互，不绑定状态。
          </div>

          {actions.length === 0 ? (
            <div className="empty">这只宠物还没有动作</div>
          ) : view === 'grid' ? (
            <div className="act-grid">
              {actions.map((a, i) => (
                <ActionCard key={a.action} a={a} idx={i} states={states} onChange={patchAction} petId={selected.id} />
              ))}
            </div>
          ) : (
            <div className="act-list">
              {actions.map((a, i) => (
                <ActionRowItem key={a.action} a={a} idx={i} states={states} onChange={patchAction} petId={selected.id} />
              ))}
            </div>
          )}

          <div className="save-row">
            <button className="btn primary" disabled={!dirty} onClick={saveActions}>
              保存{dirty ? '（有修改）' : ''}
            </button>
            {!dirty && <span className="muted small">改一改上面的归属、状态或概率再保存</span>}
          </div>
        </div>

        {/* 从视频导入动作：mp4 绿幕 → 自动转 webm（异步） */}
        <div className="card">
          <div className="section-h">＋ 导入动作（mp4 绿幕 → 自动转 webm）</div>
          <div className="muted small" style={{ marginBottom: 12 }}>
            上传一个绿幕 mp4，程序自动抠像 / 去绿边 / 归一化转成可用 webm 并注册为动作（异步，可看进度）。
          </div>
          <div
            className={`empty small dropzone${impDrag ? ' drag' : ''}`}
            onClick={() => impFileRef.current?.click()}
            onDragOver={(e) => { e.preventDefault(); setImpDrag(true) }}
            onDragLeave={() => setImpDrag(false)}
            onDrop={(e) => { e.preventDefault(); setImpDrag(false); const f = e.dataTransfer.files[0]; if (f) pickImpFile(f) }}
            style={{ padding: 22 }}
          >
            {impFile ? `已选择：${impFile.name}` : '点击或拖拽 mp4 绿幕视频到这里'}
          </div>
          <input
            ref={impFileRef}
            type="file"
            accept="video/mp4,.mp4"
            style={{ display: 'none' }}
            onChange={(e) => {
              if (e.target.files?.[0]) pickImpFile(e.target.files[0])
              e.target.value = ''
            }}
          />
          <div className="import-form">
            <div className="imp-field">
              <div className="label">动作名</div>
              <input type="text" value={impName} onChange={(e) => setImpName(e.target.value)} placeholder="如：开心蹦跳" />
            </div>
            <div className="imp-field">
              <div className="label">归属</div>
              <div className="seg">
                {(Object.keys(OWNER_LABEL) as Owner[]).map((o) => (
                  <button key={o} className={impOwner === o ? 'on' : ''} onClick={() => setImpOwner(o)}>{OWNER_LABEL[o]}</button>
                ))}
              </div>
            </div>
            <button className="btn primary" disabled={!impFile || !impName.trim() || impBusy} onClick={submitVideo}>
              {impBusy ? '提交中…' : '开始转换并入队'}
            </button>
          </div>
          {jobs.length > 0 && (
            <div className="jobs">
              <div className="label">转换作业</div>
              {jobs.map((j) => <JobRow key={j.id} j={j} />)}
            </div>
          )}
        </div>
      </>
    )
  }

  // ---------- 列表视图 ----------
  const list = pets ?? []
  return (
    <>
      {msg && <div className={`msg ${msg.ok ? 'ok' : 'err'}`}>{msg.text}</div>}

      <div className="card">
        <div className="block-head">
          <div>
            <div style={{ fontWeight: 600 }}>我的宠物</div>
            <div className="muted small">点击某只宠物进入它的动作管理；或从 zip 导入一只</div>
          </div>
          <div className="import-actions">
            <div className="seg">
              <button className={view === 'grid' ? 'on' : ''} onClick={() => setView('grid')}>▦ 网格</button>
              <button className={view === 'list' ? 'on' : ''} onClick={() => setView('list')}>☰ 列表</button>
            </div>
            <button className="btn primary" onClick={() => fileRef.current?.click()} disabled={busy}>
              ＋ 导入宠物
            </button>
            <button className="btn" onClick={() => setVideoWizard((v) => !v)}>
              🎬 从视频新建宠物
            </button>
          </div>
        </div>

        {list.length === 0 ? (
          <div
            className={`empty dropzone${drag ? ' drag' : ''}`}
            onClick={() => !busy && fileRef.current?.click()}
            onDragOver={(e) => { e.preventDefault(); setDrag(true) }}
            onDragLeave={() => setDrag(false)}
            onDrop={(e) => { e.preventDefault(); setDrag(false); if (!busy && e.dataTransfer.files[0]) upload(e.dataTransfer.files[0]) }}
          >
            <div style={{ fontSize: 34 }}>📦</div>
            <div style={{ fontWeight: 600 }}>{busy ? '正在导入…' : '还没有宠物'}</div>
            <div className="muted small">点击或拖拽一个 zip 素材包到这里</div>
          </div>
        ) : view === 'grid' ? (
          <div className="pet-grid">
            {list.map((p) => (
              <div key={p.id} className={`pet${p.is_current ? ' current' : ''}`} onClick={() => openPet(p.id)}>
                <PetImage pet={p} className="pet-cover" />
                <div className="pet-name">
                  {p.display_name}
                  {p.is_current && <span className="badge green"><span className="dot" />当前</span>}
                </div>
                <div className="small muted">{p.video_count} 个动作</div>
                <div className="pet-btns" onClick={(e) => e.stopPropagation()}>
                  {!p.is_current && <button className="btn sm primary" onClick={() => doSwitch(p.id)}>设为当前</button>}
                  <button className="btn sm danger" onClick={() => doDelete(p)}>删除</button>
                </div>
              </div>
            ))}
          </div>
        ) : (
          <div className="pet-list">
            {list.map((p) => (
              <div key={p.id} className={`row${p.is_current ? ' current' : ''}`} onClick={() => openPet(p.id)}>
                <PetImage pet={p} className="row-ph" />
                <div style={{ flex: 1 }}>
                  <div className="pet-name">
                    {p.display_name}
                    {p.is_current && <span className="badge green"><span className="dot" />当前</span>}
                  </div>
                  <div className="small muted">{p.video_count} 个动作 · 待机：{p.idle_action ?? '—'}</div>
                </div>
                <div className="pet-btns" onClick={(e) => e.stopPropagation()}>
                  {!p.is_current && <button className="btn sm primary" onClick={() => doSwitch(p.id)}>设为当前</button>}
                  <button className="btn sm danger" onClick={() => doDelete(p)}>删除</button>
                </div>
              </div>
            ))}
          </div>
        )}

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
      </div>

      {videoWizard && (
        <VideoImportWizard
          vzStaging={vzStaging}
          vzBusy={vzBusy}
          vzDrag={vzDrag}
          vzFileRef={vzFileRef}
          vzName={vzName}
          vzActions={vzActions}
          vzIdle={vzIdle}
          vzJob={vzJob}
          setVzDrag={setVzDrag}
          setVzName={setVzName}
          setVzActions={setVzActions}
          setVzIdle={setVzIdle}
          onPickFile={(f) => uploadVideoZip(f)}
          onStart={startVideoConvert}
          onCancel={() => {
            setVideoWizard(false)
            setVzStaging(null)
            setVzJob(null)
            setVzJobId(null)
          }}
        />
      )}
    </>
  )
}

function ActionCard({ a, idx, states, onChange, petId }: {
  a: EditAction; idx: number; states: StateDef[]; onChange: (i: number, fn: (a: EditAction) => EditAction) => void; petId: string
}) {
  return (
    <div className={`act${!a.enabled ? ' off' : ''}`}>
      <video src={api.webmUrl(petId, a.action)} loop muted playsInline />
      <div className="body">
        <input
          className="act-name"
          type="text"
          value={a.display_name}
          onChange={(e) => onChange(idx, (x) => ({ ...x, display_name: e.target.value }))}
        />
        <OwnerPicker a={a} onChange={(owner) => onChange(idx, (x) => ({ ...x, owner }))} />
        {a.owner === 'state' && <StatePicker a={a} states={states} onChange={(bindings) => onChange(idx, (x) => ({ ...x, bindings }))} />}
        <div className="act-foot">
          <span className="muted small">启用</span>
          <Toggle
            on={a.enabled}
            onClick={() => onChange(idx, (x) => ({ ...x, enabled: !x.enabled }))}
          />
        </div>
      </div>
    </div>
  )
}

function ActionRowItem({ a, idx, states, onChange, petId }: {
  a: EditAction; idx: number; states: StateDef[]; onChange: (i: number, fn: (a: EditAction) => EditAction) => void; petId: string
}) {
  return (
    <div className="act-table-row">
      <div className="act-thumb">
        <video src={api.webmUrl(petId, a.action)} loop muted playsInline />
      </div>
      <div className="act-col">
        <input className="act-name" type="text" value={a.display_name}
          onChange={(e) => onChange(idx, (x) => ({ ...x, display_name: e.target.value }))} />
      </div>
      <div className="act-col"><OwnerPicker a={a} onChange={(owner) => onChange(idx, (x) => ({ ...x, owner }))} /></div>
      {a.owner === 'state' && (
        <div className="act-col wide">
          <StatePicker a={a} states={states} compact onChange={(bindings) => onChange(idx, (x) => ({ ...x, bindings }))} />
        </div>
      )}
      <div className="act-col">
        <Toggle on={a.enabled} onClick={() => onChange(idx, (x) => ({ ...x, enabled: !x.enabled }))} />
      </div>
    </div>
  )
}

function OwnerPicker({ a, onChange }: { a: EditAction; onChange: (o: Owner) => void }) {
  return (
    <div className="seg">
      {(Object.keys(OWNER_LABEL) as Owner[]).map((o) => (
        <button key={o} className={a.owner === o ? 'on' : ''} onClick={() => onChange(o)}>{OWNER_LABEL[o]}</button>
      ))}
    </div>
  )
}

function StatePicker({ a, states, onChange, compact }: {
  a: EditAction; states: StateDef[]; onChange: (b: Record<string, Binding>) => void; compact?: boolean
}) {
  const enabledStates = states.filter((s) => s.enabled)
  if (enabledStates.length === 0) return <div className="muted small">（暂无可用状态）</div>

  const picked = enabledStates.filter((s) => a.bindings[s.id]?.enabled)

  function toggleState(id: string) {
    const b = { ...a.bindings }
    const cur = b[id]
    b[id] = cur ? { ...cur, enabled: !cur.enabled } : { weight: 1, enabled: true }
    onChange(b)
  }
  function setWeight(id: string, weight: number) {
    onChange({ ...a.bindings, [id]: { weight, enabled: true } })
  }

  return (
    <div className="st-pick">
      <div className="label">所属状态（可多选）</div>
      <div className="st-checks">
        {enabledStates.map((s) => (
          <label className="chk" key={s.id}>
            <input type="checkbox" checked={!!a.bindings[s.id]?.enabled} onChange={() => toggleState(s.id)} />
            {s.name}
          </label>
        ))}
      </div>
      {picked.length > 0 && (
        <div className="st-weights">
          {picked.map((s) => (
            <div className="st-p" key={s.id}>
              <span>{s.name}</span>
              <input type="range" min={0} max={100} value={Math.round((a.bindings[s.id]?.weight ?? 1) * 100)}
                onChange={(e) => setWeight(s.id, Number(e.target.value) / 100)} />
              <b>{Math.round((a.bindings[s.id]?.weight ?? 1) * 100)}%</b>
            </div>
          ))}
        </div>
      )}
      {!compact && <div className="muted small">未勾选的状态即不出现在该状态下</div>}
    </div>
  )
}

function Toggle({ on, onClick }: { on: boolean; onClick: () => void }) {
  return <span className={`switch${on ? ' on' : ''}`} onClick={onClick} />
}

function JobRow({ j }: { j: ConvertJob }) {
  const action = j.src.split('/').pop()?.replace(/\.src\.mp4$/i, '') ?? `作业#${j.id}`
  const label = { queued: '排队中', running: '转换中', done: '完成', error: '失败' }[j.status] ?? j.status
  const pct = Math.round((j.progress ?? 0) * 100)
  return (
    <div className="job-row">
      <div className="job-name">{action}</div>
      <div className="job-progress">
        <div className="job-bar"><div className="job-fill" style={{ width: `${pct}%` }} /></div>
        <span className={`badge ${j.status === 'done' ? 'green' : j.status === 'error' ? 'red' : 'blue'}`}>
          <span className="dot" />{label}{j.status === 'running' ? ` ${pct}%` : ''}
        </span>
      </div>
      {j.error && <div className="muted small job-error">{j.error}</div>}
    </div>
  )
}

// ---- 从视频新建宠物向导（§7.3） ----
function VideoImportWizard(props: {
  vzStaging: { pet_id: string; videos: string[] } | null
  vzBusy: boolean
  vzDrag: boolean
  vzFileRef: RefObject<HTMLInputElement>
  vzName: string
  vzActions: Record<string, string>
  vzIdle: string
  vzJob: PetImportJob | null
  setVzDrag: (b: boolean) => void
  setVzName: (s: string) => void
  setVzActions: (a: Record<string, string>) => void
  setVzIdle: (s: string) => void
  onPickFile: (f: File) => void
  onStart: () => void
  onCancel: () => void
}) {
  const {
    vzStaging, vzBusy, vzDrag, vzFileRef, vzName, vzActions, vzIdle, vzJob,
    setVzDrag, setVzName, setVzActions, setVzIdle, onPickFile, onStart, onCancel,
  } = props

  return (
    <div className="card">
      <div className="section-h">🎬 从视频新建宠物（视频包 → 自动建宠）</div>
      <div className="muted small" style={{ marginBottom: 12 }}>
        上传一个只含源视频（.mp4/.mov）的 zip，程序会以共享锚点统一归一化，从待机动画自动提取全身照并注册整只宠物。
      </div>

      {vzJob ? (
        <div>
          <div className="job-row">
            <div className="job-name">批量建宠 {vzJob.current_action || '…'}</div>
            <div className="job-progress">
              <div className="job-bar">
                <div className="job-fill" style={{ width: `${vzJob.total ? Math.round((vzJob.done / vzJob.total) * 100) : 0}%` }} />
              </div>
              <span className={`badge ${vzJob.status === 'done' ? 'green' : vzJob.status === 'error' ? 'red' : 'blue'}`}>
                <span className="dot" />
                {vzJob.status === 'running' ? `转换中 ${vzJob.done}/${vzJob.total}` : vzJob.status === 'done' ? '完成' : '失败'}
              </span>
            </div>
          </div>
          {vzJob.error && <div className="muted small job-error">{vzJob.error}</div>}
        </div>
      ) : !vzStaging ? (
        <>
          <div
            className={`empty small dropzone${vzDrag ? ' drag' : ''}`}
            onClick={() => vzFileRef.current?.click()}
            onDragOver={(e) => { e.preventDefault(); setVzDrag(true) }}
            onDragLeave={() => setVzDrag(false)}
            onDrop={(e) => { e.preventDefault(); setVzDrag(false); const f = e.dataTransfer.files[0]; if (f && !vzBusy) onPickFile(f) }}
            style={{ padding: 22 }}
          >
            {vzBusy ? '正在解析视频包…' : '点击或拖拽视频包 zip 到这里'}
          </div>
          <input
            ref={vzFileRef}
            type="file"
            accept=".zip,application/zip"
            style={{ display: 'none' }}
            disabled={vzBusy}
            onChange={(e) => {
              if (e.target.files?.[0]) onPickFile(e.target.files[0])
              e.target.value = ''
            }}
          />
          <button className="btn ghost" onClick={onCancel}>取消</button>
        </>
      ) : (
        <div className="import-form" style={{ flexDirection: 'column', alignItems: 'stretch' }}>
          <div className="imp-field">
            <div className="label">宠物名</div>
            <input type="text" value={vzName} onChange={(e) => setVzName(e.target.value)} placeholder="如：蓝发女仆" />
          </div>
          <div className="imp-field">
            <div className="label">逐视频动作名（默认=文件名，可改）</div>
            <div className="vz-actions">
              {vzStaging.videos.map((file) => (
                <div key={file} className="vz-action">
                  <span className="muted small" style={{ width: '40%', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                    {file}
                  </span>
                  <input type="text" value={vzActions[file] ?? ''} onChange={(e) => setVzActions({ ...vzActions, [file]: e.target.value })} />
                </div>
              ))}
            </div>
          </div>
          <div className="imp-field">
            <div className="label">指定待机动画（体型基准 / 全身照来源）</div>
            <div className="seg wrap">
              {vzStaging.videos.map((file) => (
                <button key={file} className={vzIdle === file ? 'on' : ''} onClick={() => setVzIdle(file)}>
                  {vzActions[file] || file}
                </button>
              ))}
            </div>
          </div>
          <div className="imp-actions">
            <button className="btn ghost" onClick={onCancel}>返回</button>
            <button className="btn primary" disabled={vzBusy || !vzName.trim() || !vzIdle} onClick={onStart}>
              {vzBusy ? '启动中…' : '开始转换并建宠'}
            </button>
          </div>
        </div>
      )}
    </div>
  )
}
