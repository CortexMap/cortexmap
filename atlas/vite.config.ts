import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

export default defineConfig({
  plugins: [react()],
  server: {
    port: 3001,
    proxy: {
      '/orch/api': {
        target: 'https://capstone.ssdd.dev',
        changeOrigin: true,
        secure: true,
      },
      '/allen-meshes': {
        target: 'http://download.alleninstitute.org/informatics-archive/current-release/mouse_ccf/annotation/ccf_2017/structure_meshes',
        changeOrigin: true,
        rewrite: (path) => path.replace(/^\/allen-meshes/, ''),
      },
    },
  },
  build: {
    outDir: 'dist',
    sourcemap: false,
    rollupOptions: {
      output: {
        manualChunks: {
          three: ['three'],
        },
      },
    },
  },
});
