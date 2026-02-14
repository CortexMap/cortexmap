import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'

export default defineConfig({
  plugins: [react()],
  base: '/fetcher-fe/',
  server: {
    port: 3000,
    open: true,
    host: true,
    proxy: {
      '/fetcher-be': {
        target: process.env.VITE_BACKEND_URL || 'https://capstone.ssdd.dev',
        changeOrigin: true,
        secure: false
      }
    }
  },
  preview: {
    port: 3000,
    host: true,
    allowedHosts: ['capstone.ssdd.dev', 'localhost']
  }
})
