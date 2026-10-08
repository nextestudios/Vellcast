import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { App } from './App';
import { WatchApp } from './WatchApp';
import { isWatchPath } from './watch';
import './styles.css';

const root = document.getElementById('root');
if (root) {
  createRoot(root).render(
    <StrictMode>
      {isWatchPath(location.pathname, location.hash) ? <WatchApp /> : <App />}
    </StrictMode>,
  );
}
