import { useRef, useState } from 'react'
import { api } from '../api'

interface ImportData {
  id: string
  display_name: string
  videos: number
  warnings: string[]
  character?: string | null
}

interface ImportOutcome {
  ok: boolean
  text: string
  data?: ImportData
}

export default function ImportPage() {
  const [drag, setDrag] = useState(false)
  const [busy, setBusy] = useState(false)
  const [outcome, setOutcome] = useState<ImportOutcome | null>(null)
  const fileRef = useRef<HTMLInputElement>(null)

  async function upload(f: File) {
    if (!f.name.toLowerCase().endsWith('.zip')) {
      setOutcome({ ok: false, text: '请选择 .zip 素材包' })
      return
    }
    setBusy(true)
    setOutcome(null)
    try {
      const r = (await api.importZip(f)) as { ok: boolean; data?: ImportData; error?: string }
      if (r.ok && r.data) {
        const d = r.data
        setOutcome({
          ok: true,
          text: `导入成功`,
          data: d,
        })
      } else {
        setOutcome({ ok: false, text: r.error ?? '导入失败' })
      }
    } catch (e) {
      setOutcome({ ok: false, text: `请求失败: ${e}` })
    }
    setBusy(false)
  }

  return (
    <>
      <div className="card">
        <h2>
          导入素材包（zip）
          <span className="hint">发布物仅二进制，素材一律经此导入</span>
        </h2>
        <div
          className={'drop' + (drag ? ' drag' : '') + (busy ? ' busy' : '')}
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
          {busy ? '导入中，请稍候…（校验 / 解压 / 热加载）' : '点击或拖拽 zip 文件到此处'}
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
        <p className="muted" style={{ marginBottom: 0 }}>
          素材包规范：zip 根目录即角色包 —— <code>manifest.json</code> + <code>videos/</code>
          （VP9+alpha webm）。导入后校验合法性、解压到素材根并热加载（不重启），
          导入的角色自动成为当前角色。
        </p>
      </div>

      {outcome && (
        <div className="card">
          <h2>导入结果</h2>
          <div className={'msg ' + (outcome.ok ? 'ok' : 'err')}>{outcome.text}</div>
          {outcome.ok && outcome.data && (
            <dl className="kv" style={{ marginTop: 12 }}>
              <dt>角色 id</dt>
              <dd>{outcome.data.id}</dd>
              <dt>显示名</dt>
              <dd>{outcome.data.display_name}</dd>
              <dt>视频数</dt>
              <dd>{outcome.data.videos}</dd>
              {outcome.data.character != null && (
                <>
                  <dt>当前角色</dt>
                  <dd>{outcome.data.character}</dd>
                </>
              )}
              {outcome.data.warnings.length > 0 && (
                <>
                  <dt>警告</dt>
                  <dd>
                    {outcome.data.warnings.map((w, i) => (
                      <div key={i}>⚠ {w}</div>
                    ))}
                  </dd>
                </>
              )}
            </dl>
          )}
        </div>
      )}
    </>
  )
}
