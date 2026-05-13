import 'virtual:uno.css'
import './components/ui/animations.css'
import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import './styles/index.css'
import App from './App'

if (!import.meta.env.DEV) {
  document.addEventListener('contextmenu', (e) => e.preventDefault());

  document.addEventListener('keydown', (e) => {
    const isDevToolsShortcut =
      (e.key === 'F12') ||
      (e.ctrlKey && e.shiftKey && (e.key === 'I' || e.key === 'i')) ||
      (e.metaKey && e.altKey && (e.key === 'I' || e.key === 'i')) ||
      (e.ctrlKey && e.shiftKey && (e.key === 'C' || e.key === 'c')) ||
      (e.ctrlKey && e.shiftKey && (e.key === 'J' || e.key === 'j'));

    if (isDevToolsShortcut) {
      e.preventDefault();
    }
  });
}

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <App />
  </StrictMode>,
)
