import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { ActivityApp } from './ActivityApp';
import { App } from './App';
import { insideDiscord } from './discord';
import { WatchApp } from './WatchApp';
import { isWatchPath } from './watch';
import './styles.css';

const root = document.getElementById('root');
if (root) {
  createRoot(root).render(
    <StrictMode>
      {insideDiscord() && !new URLSearchParams(location.search).has('stage0') ? (
        <ActivityApp />
      ) : isWatchPath(location.pathname, location.hash) ? (
        <WatchApp />
      ) : (
        <App />
      )}
    </StrictMode>,
  );
}
