import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'

// base 固定 '/'：产物内嵌后由桌宠 HTTP 服务从根路径托管（/assets/* 由服务映射）。
export default defineConfig({
  plugins: [react()],
  base: '/',
  build: {
    outDir: 'dist',
    assetsDir: 'assets',
    // 产物会经 include_bytes! 内嵌进二进制，越小越好
    minify: 'esbuild',
    target: 'es2020',
  },
})
