import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { App } from '@ariadne/ui';
import '@ariadne/ui/styles.css';

import { TauriAgentClient } from './tauriAgentClient';

const root = document.getElementById('root');
if (!root) {
  throw new Error('Ariadne root element is missing');
}

createRoot(root).render(
  <StrictMode>
    <App client={new TauriAgentClient()} />
  </StrictMode>,
);
