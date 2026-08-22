import { useState } from 'react'
import { api } from './api'
import DashboardPage from './pages/DashboardPage'
import PetsPage from './pages/PetsPage'
import StatesPage from './pages/StatesPage'
import SettingsPage from './pages/SettingsPage'

type Nav = 'overview' | 'pets' | 'states' | 'settings'

const NAVS: { id: Nav; label: string; icon: string }[] = [
  { id: 'overview', label: '总览', icon: '◉' },
  { id: 'pets', label: '宠物管理', icon: '♞' },
  { id: 'states', label: '状态配置', icon: '⏱' },
  { id: 'settings', label: '设置', icon: '⚙' },
]

const TITLES: Record<Nav, [string, string]> = {
  overview: ['总览', '看一眼桌宠现在怎么样'],
  pets: ['宠物管理', '管理桌宠、动作与全身照'],
  states: ['状态配置', '全局定义空闲 / 活跃 / 午休'],
  settings: ['设置', '外观、行为与视频转换默认参数'],
}

export default function App() {
  const [nav, setNav] = useState<Nav>('overview')

  return (
    <div className="app">
      <aside className="sidebar">
        <div className="brand">
          <div className="logo">D</div>
          <b>DeskPet 控制台</b>
        </div>
        <nav className="nav">
          {NAVS.map((n) => (
            <a
              key={n.id}
              className={nav === n.id ? 'active' : ''}
              onClick={() => setNav(n.id)}
            >
              <span className="ic">{n.icon}</span>
              {n.label}
            </a>
          ))}
        </nav>
        <div className="side-foot">本地运行 · 数据仅存本机</div>
      </aside>

      <main className="main">
        <div className="topbar">
          <div>
            <h1>{TITLES[nav][0]}</h1>
            <div className="sub">{TITLES[nav][1]}</div>
          </div>
          <div className="top-actions">
            <PetSwitcher />
          </div>
        </div>

        {nav === 'overview' && <DashboardPage />}
        {nav === 'pets' && <PetsPage />}
        {nav === 'states' && <StatesPage />}
        {nav === 'settings' && <SettingsPage />}
      </main>
    </div>
  )
}

function PetSwitcher() {
  const [pets, setPets] = useState<{ id: string; display_name: string; is_current: boolean }[]>([])
  const [loaded, setLoaded] = useState(false)
  const current = pets.find((p) => p.is_current)

  if (!loaded) {
    api.pets().then((r) => {
      if (r.ok && r.data) setPets(r.data)
      setLoaded(true)
    })
    return <span className="muted small">…</span>
  }
  if (pets.length === 0) {
    return <span className="muted small">还没有宠物</span>
  }
  return (
    <span className="top-switch">
      <span className="muted small">当前宠物</span>
      <select
        value={current?.id ?? ''}
        onChange={(e) => {
          const id = e.target.value
          api.switchPet(id).then(() => {
            // 简单刷新：让 App 重新挂载一次整个界面依赖的状态
            window.location.reload()
          })
        }}
      >
        {pets.map((p) => (
          <option key={p.id} value={p.id}>
            {p.display_name}
          </option>
        ))}
      </select>
    </span>
  )
}
