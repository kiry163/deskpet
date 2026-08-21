import { useState } from 'react'
import DashboardPage from './pages/DashboardPage'
import PetsPage from './pages/PetsPage'
import SettingsPage from './pages/SettingsPage'

type Tab = 'home' | 'pets' | 'settings'

const TABS: { id: Tab; label: string }[] = [
  { id: 'home', label: '主页' },
  { id: 'pets', label: '我的桌宠' },
  { id: 'settings', label: '设置' },
]

export default function App() {
  const [tab, setTab] = useState<Tab>('home')

  return (
    <div>
      <header className="topbar">
        <span className="logo">🐱 DeskPet</span>
        <nav>
          {TABS.map((t) => (
            <button
              key={t.id}
              className={tab === t.id ? 'active' : ''}
              onClick={() => setTab(t.id)}
            >
              {t.label}
            </button>
          ))}
        </nav>
      </header>
      <main>
        {tab === 'home' && <DashboardPage />}
        {tab === 'pets' && <PetsPage />}
        {tab === 'settings' && <SettingsPage />}
      </main>
    </div>
  )
}
