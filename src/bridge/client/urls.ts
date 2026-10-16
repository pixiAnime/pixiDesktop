/**
 * Desktop URL builders — the app's own scheme, not a localhost socket.
 *
 * Every URL is built off `config.pixiClient.baseUrl`, which the config shadow
 * points at the app's protocol base (`pixi://localhost`, or
 * `http://pixi.localhost` on Windows) — so `/request`, `/m3u8` and
 * `/m3u8/segment` land in Rust's scheme handler.
 *
 * pixiWeb's versions return the target unchanged: a browser can hand a provider
 * URL straight to hls.js and let the extension deal with headers and CORS. A
 * desktop webview cannot — `media-src` allows only this scheme, and Rust is
 * where SSRF validation, playlist rewriting, `Range` and Save-to-Downloads
 * live — so the proxied shapes are kept here. Only `@/extensions`' playback
 * adapter uses them (see `src/shadow/extensions/playbackAdapter.ts`).
 */
import { config } from '@/config'

export interface ProxyUrlOptions {
  /** Wrap 2xx text responses in the JSON envelope (docs §6.7). */
  envelope?: boolean
  /** Media elements cannot set headers — append the token as a query param. */
  token?: string
}

function buildQuery(target: string | URL, extra?: Record<string, string>): URLSearchParams {
  const params = new URLSearchParams({ url: String(target) })
  if (extra) {
    for (const [key, value] of Object.entries(extra)) params.set(key, value)
  }
  return params
}

/** The desktop token is empty (the scheme handler is same-process); a caller
 *  override still gets through, so the shape matches pixiWeb's. */
function withToken(params: URLSearchParams, token?: string): string {
  const effective = token ?? config.pixiClient.token
  if (effective) params.set('token', effective)
  return params.toString()
}

/** Generic passthrough proxy URL (§6.5 / §6.6 query form) — Range-safe media. */
export function bridgeRequestUrl(target: string | URL, options: ProxyUrlOptions = {}): string {
  const params = buildQuery(target)
  if (options.envelope) params.set('envelope', '1')
  return `${config.pixiClient.baseUrl}/request?${withToken(params, options.token)}`
}

/** Rewritten HLS playlist URL — master or media (§6.8). */
export function bridgePlaylistUrl(target: string | URL, options: ProxyUrlOptions = {}): string {
  return `${config.pixiClient.baseUrl}/m3u8?${withToken(buildQuery(target), options.token)}`
}

/** Segment / key / init-segment / nested-playlist URL (§6.9). */
export function bridgeSegmentUrl(target: string | URL, options: ProxyUrlOptions = {}): string {
  return `${config.pixiClient.baseUrl}/m3u8/segment?${withToken(buildQuery(target), options.token)}`
}

/** Reverse: recover the upstream target from any scheme URL. */
export function bridgeTargetOf(bridgeUrl: string): string | null {
  try {
    return new URL(bridgeUrl).searchParams.get('url')
  } catch {
    return null
  }
}
