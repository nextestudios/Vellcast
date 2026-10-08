import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

// Em desenvolvimento, /ws e /api são encaminhados ao servidor da ETAPA 0.
const server = process.env.VELLCAST_SERVER ?? 'http://127.0.0.1:8787';

export default defineConfig({
  plugins: [react()],
  server: {
    port: 5173,
    proxy: {
      '/api': server,
      '/ws': { target: server, ws: true },
    },
  },
  build: { target: 'es2022', sourcemap: true },
});
