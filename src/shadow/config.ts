/**
 * Desktop config — pixiWeb's config plus two desktop-only facts.
 *
 *  - `pixiClient.baseUrl` becomes the app's own protocol base. On desktop there
 *    is no pixiClient: the same URL shapes (`/request`, `/m3u8`) are served by
 *    Rust through the custom URI scheme (src-tauri/src/protocol.rs).
 *  - `isDesktop` exists for presentation-only decisions (never for transport —
 *    transport is swapped wholesale by the module shadows in vite.config.ts).
 *
 * The base is resolved lazily: `src/main.ts` asks Rust for the platform-correct
 * value before the React app is imported, and the UA fallback below only covers
 * the case where that failed (e.g. dev server opened in a plain browser).
 */
import { config as base } from '../../../pixiWeb/src/config/index'

/** Set by src/main.ts from `invoke('protocol_base')` before the app boots. */
let protocolBase = ''

export function setProtocolBase(value: string): void {
  protocolBase = value
}

/** Mirrors Tauri's own `convertFileSrc` platform rule; Rust is the source of truth. */
function detectProtocolBase(): string {
  if (protocolBase) return protocolBase
  return typeof navigator === 'object' && navigator.userAgent.includes('Windows')
    ? 'http://pixi.localhost'
    : 'pixi://localhost'
}

export const config = {
  ...base,
  isDesktop: true,
  pixiClient: {
    ...base.pixiClient,
    get baseUrl(): string {
      return detectProtocolBase()
    },
    token: '',
  },
}
