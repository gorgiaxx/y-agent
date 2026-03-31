import { GlobalProviders } from './providers/GlobalProviders';
import { MainLayout } from './layouts/MainLayout';
import { ToastProvider } from './components/ui';
import './App.css';

export default function App() {
  return (
    <ToastProvider>
      <GlobalProviders>
        <MainLayout />
      </GlobalProviders>
    </ToastProvider>
  );
}
