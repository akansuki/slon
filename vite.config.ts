import { defineConfig } from 'vitest/config';
import { svelte } from '@sveltejs/vite-plugin-svelte';

// Tauri expects a fixed dev port and must not have its console cleared.
export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  server: { port: 1420, strictPort: true, watch: { ignored: ['**/src-tauri/**', '**/crates/**'] } },
  build: { target: 'es2022', chunkSizeWarningLimit: 900 },
  test: { include: ['src/**/*.test.ts'] },
});
