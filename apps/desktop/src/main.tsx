import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { App } from '@rynna/ui';
import '@rynna/ui/styles.css';

import { TauriAgentClient } from './tauriAgentClient';

const root = document.getElementById('root');
if (!root) {
  throw new Error('Rynna root element is missing');
}

createRoot(root).render(
  <StrictMode>
    <App client={new TauriAgentClient()} />
  </StrictMode>,
);
