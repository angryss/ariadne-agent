import tailwindcss from '@tailwindcss/vite';
import react from '@vitejs/plugin-react';
import { defineConfig } from 'vite';

export default defineConfig({
  plugins: [react(), tailwindcss()],
  server: {
    port: 5173,
    strictPort: true,
    proxy: {
      '/v1': 'http://127.0.0.1:3000',
      '/healthz': 'http://127.0.0.1:3000',
    },
  },
});
