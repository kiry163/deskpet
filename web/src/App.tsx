import { useState } from 'react'
import StatePage from './pages/StatePage'
import ImportPage from './pages/ImportPage'
import ConfigPage from './pages/ConfigPage'

type Tab = 'state' | 'import' | 'config'

const TABS: { id: Tab; label: string }[] = [
  { id: 'state', label: '状态' },
  { id: 'import', label: '导入' },
  { id: 'config', label: '配置' },
]

export default function App() {
  const [tab, setTab] = useState<Tab>('state')

  return (
    <div>
      <header className="topbar">
        <span className="logo">🐱 deskpet 控制台</span>
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
        {tab === 'state' && <StatePage />}
        {tab === 'import' && <ImportPage />}
        {tab === 'config' && <ConfigPage />}
      </main>
    </div>
  )
}
