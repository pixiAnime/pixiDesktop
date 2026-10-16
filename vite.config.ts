/**
 * Vite config for the PixiAnime desktop shell.
 *
 * THE seam: the React app itself is pixiWeb's source (untouched). This config
 * points `@` at ../pixiWeb/src and shadows exactly four module paths with the
 * desktop implementations (same export surface, Tauri-backed):
 *
 *   @/config                → platform protocol base + isDesktop
 *   @/api/pixiclient        → bridge: commands instead of a localhost socket
 *   @/components/pixiclient → render nothing: the bridge is in-process
 *   @/extensions            → playback through the scheme, not the provider CDN
 *
 * tsconfig.app.json mirrors these mappings, so `pnpm check` type-checks pixiWeb's
 * own sources against the shadows — an export-surface drift fails the build.
 */
import { fileURLToPath } from 'node:url'
import tailwindcss from '@tailwindcss/vite'
import react from '@vitejs/plugin-react'
import { defineConfig, type Plugin } from 'vite'

const WEB = fileURLToPath(new URL('../pixiWeb', import.meta.url))
const SHADOW = fileURLToPath(new URL('./src/shadow', import.meta.url))
const BRIDGE = fileURLToPath(new URL('./src/bridge', import.meta.url))
const APP = fileURLToPath(new URL('./src', import.meta.url))
const APP_HTML = fileURLToPath(new URL('./index.html', import.meta.url))

/**
 * Tailwind v4 auto-detects sources from the vite root — which here includes
 * src-tauri/target (~13.6k files; no .gitignore): its scan of that tree is
 * the ~25s single-threaded stall behind every cold start ("page never
 * loads"). `source(none)` disables auto-detection; only the two real source
 * trees are scanned. Runs before the Tailwind plugin sees the file, so every
 * utility class in either app still lands in the bundle.
 */
/**
 * pixiWeb registers a service worker in production (public/sw.js) to cache a
 * hosted bundle. A desktop window serves its own assets off the local disk, so
 * caching buys nothing — and the registration outlives the app: a worker from
 * an older build keeps answering navigations after an update. pixiWeb's source
 * is never edited, so the guard is neutered here instead.
 *
 * Throws when the line is gone: a silent no-op would ship the worker instead.
 */
function noServiceWorker(): Plugin {
  const GUARD = "if (!import.meta.env.DEV && 'serviceWorker' in navigator) {"
  return {
    name: 'pixiweb-no-service-worker',
    enforce: 'pre',
    transform(code, id) {
      if (!id.replace(/\\/g, '/').endsWith('/pixiWeb/src/main.tsx')) return null
      if (!code.includes(GUARD)) {
        this.error(
          `pixiWeb's service-worker guard changed — update noServiceWorker() in vite.config.ts (${id})`,
        )
      }
      return code.replace(GUARD, 'if (false) {')
    },
  }
}

function pixiwebSources(): Plugin {
  return {
    name: 'pixiweb-sources',
    enforce: 'pre',
    transform(code, id) {
      if (!id.replace(/\\/g, '/').endsWith('/pixiWeb/src/index.css')) return null
      const sources = [`${WEB}/src`, APP, APP_HTML]
        .map((p) => `\n@source ${JSON.stringify(p)};`)
        .join('')
      return code.replace(
        /@import\s+(["'])tailwindcss\1;/,
        `@import "tailwindcss" source(none);${sources}`,
      )
    },
  }
}

export default defineConfig({
  plugins: [pixiwebSources(), noServiceWorker(), react(), tailwindcss()],
  resolve: {
    alias: [
      // The shadow's own .tsx files live in this package, but react itself is
      // a pixiWeb dependency — one copy, or hooks would see two Reacts.
      { find: 'react', replacement: `${WEB}/node_modules/react` },
      { find: 'react-dom', replacement: `${WEB}/node_modules/react-dom` },
      { find: '@/config', replacement: `${SHADOW}/config.ts` },
      { find: '@/components/pixiclient', replacement: `${SHADOW}/components-pixiclient` },
      { find: '@/api/pixiclient', replacement: BRIDGE },
      // Only the extensions *barrel* is shadowed (the players import
      // preparePlayback from it); every `@/extensions/…` subpath stays
      // pixiWeb's. The subpath entry must be a regex: Vite strips a trailing
      // slash from string pairs, which would make both entries match the barrel.
      { find: /^@\/extensions\//, replacement: `${WEB}/src/extensions/` },
      { find: '@/extensions', replacement: `${SHADOW}/extensions/index.ts` },
      { find: '@', replacement: `${WEB}/src` },
    ],
  },
  // pixiWeb owns the public assets (favicon etc.).
  publicDir: `${WEB}/public`,
  server: {
    port: 1420,
    strictPort: true,
    // The app source lives outside this root — allow reading it.
    fs: { allow: [fileURLToPath(new URL('.', import.meta.url)), WEB] },
    // Pre-transform the boot graph the moment the server is up: it runs in
    // parallel with cargo's compile, so the window opens onto warm modules
    // instead of ~27s of on-demand transforms (reads as "page never loads").
    warmup: {
      clientFiles: ['./src/main.ts', `${WEB}/src/main.tsx`, `${WEB}/src/pages/Home/Home.tsx`],
    },
  },
  build: {
    target: 'es2022', // top-level await in the desktop entry
    outDir: fileURLToPath(new URL('./dist', import.meta.url)),
    emptyOutDir: true,
  },
})
