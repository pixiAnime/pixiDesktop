/**
 * Desktop bridge transport — pixiWeb's `bridgeFetch` contract, executed by
 * Tauri commands instead of `fetch()` against a localhost socket.
 *
 * The four path shapes pixiWeb builds are translated 1:1:
 *
 *   GET  /request?url=…[&envelope=1] + init   → http_request (pass-through form)
 *   POST /request {…}                         → http_request (structured form)
 *   GET  /m3u8?url=…                          → fetch_playlist
 *   POST /m3u8 {url, headers}                 → fetch_playlist (protected HLS)
 *
 * Rust rejects with `{error, message, status}` using pixiClient's error codes,
 * which become `BridgeError` here; a failed IPC call is the desktop equivalent
 * of a dead bridge (`BridgeOfflineError`).
 */
import { invoke } from '@tauri-apps/api/core'
import { config } from '@/config'
import { pixiHealth } from '../health'
import { BridgeError, BridgeOfflineError, classifyBridgeResponse, isBridgeErrorCode } from './errors'

export interface BridgeFetchInit extends RequestInit {
  /** Override the configured token (rarely needed). */
  token?: string
}

/** Mirrors the JSON form of pixiClient's `/request` (docs §6.6). */
interface RequestPayload {
  url: string
  method: string
  headers: Record<string, string> | null
  body: string | null
  content_type: string | null
  envelope: boolean
}

interface PlaylistPayload {
  url: string
  headers: Record<string, string> | null
}

/** Response shape every network command returns. */
export interface PlainHttpResult {
  status: number
  headers: Record<string, string>
  text: string
}

interface BridgeErrorPayload {
  error: string
  message: string
  status: number
}

/**
 * The webview *is* the browser here, so the real User-Agent rides along on
 * every upstream call. Without it Rust falls back to a fabricated
 * desktop-Chrome identity, and hosts that fingerprint bots refuse that
 * outright — Codeberg answered an extension download with
 * `403 Access denied, old Chrome version`. pixiClient sent `navigator.userAgent`
 * for exactly the same reason; a caller that named its own UA keeps it.
 */
function withUserAgent(headers: Record<string, string> | null): Record<string, string> | null {
  if (typeof navigator !== 'object' || !navigator.userAgent) return headers
  const named = headers ?? {}
  if (Object.keys(named).some((name) => name.toLowerCase() === 'user-agent')) return headers
  return { 'User-Agent': navigator.userAgent, ...named }
}

/**
 * Headers pixiWeb set for the upstream call. `Content-Type` is lifted out so
 * Rust can send it even when the structured payload replaces the header map.
 */
function toHeaderMap(init: BridgeFetchInit): Record<string, string> | null {
  const headers = new Headers(init.headers)
  const out: Record<string, string> = {}
  headers.forEach((value, name) => {
    out[name] = value
  })
  return withUserAgent(Object.keys(out).length > 0 ? out : null)
}

function asResult(value: unknown): PlainHttpResult {
  const res = value as Partial<PlainHttpResult> | null
  return {
    status: typeof res?.status === 'number' ? res.status : 502,
    headers: res?.headers && typeof res.headers === 'object' ? res.headers : {},
    text: typeof res?.text === 'string' ? res.text : '',
  }
}

/**
 * Wraps a command result in a real `Response` (status must be 100–599 and no
 * body is allowed on 204/205/304), then runs pixiWeb's classifier so an error
 * body still surfaces as `BridgeError`.
 */
function toResponse(value: unknown): Promise<Response> {
  const { status, headers, text } = asResult(value)
  const safeStatus = Math.min(Math.max(status, 200), 599)
  const bodyless = safeStatus === 204 || safeStatus === 205 || safeStatus === 304
  const out = new Headers()
  for (const [name, value_] of Object.entries(headers)) {
    // Body was decoded by Rust — transport-level metadata no longer applies.
    if (name === 'content-length' || name === 'content-encoding') continue
    out.set(name, value_)
  }
  const response = new Response(bodyless ? null : text, { status: safeStatus, headers: out })
  return classifyBridgeResponse(response)
}

/** Invoke a command and normalize its rejection to the bridge error taxonomy. */
async function call<T>(command: string, args: Record<string, unknown>): Promise<T> {
  try {
    const value = await invoke<T>(command, args)
    // Same bookkeeping as the web client: any answered bridge call ⇒ online.
    pixiHealth.getState().markOnline()
    return value
  } catch (err) {
    const payload = err as Partial<BridgeErrorPayload> | null
    if (payload && typeof payload === 'object' && isBridgeErrorCode(payload.error)) {
      throw new BridgeError(payload.error, String(payload.message ?? payload.error), payload.status ?? 502)
    }
    // IPC itself failed — the desktop twin of "the bridge is not running".
    pixiHealth.getState().markOffline()
    throw new BridgeOfflineError()
  }
}

function parseJson<T>(raw: BodyInit | null | undefined): T {
  const text = typeof raw === 'string' ? raw : ''
  try {
    return JSON.parse(text) as T
  } catch {
    throw new BridgeError('REQUEST_TOO_LARGE', 'The bridge payload was malformed.', 400)
  }
}

export async function bridgeFetch(path: string, init: BridgeFetchInit = {}): Promise<Response> {
  const url = new URL(path, 'http://bridge.invalid')
  const method = (init.method ?? 'GET').toUpperCase()

  try {
    // POST /m3u8 — protected playlist: upstream headers applied by Rust.
    if (url.pathname === '/m3u8' && method === 'POST') {
      const payload = parseJson<PlaylistPayload>(init.body)
      const text = await call<string>('fetch_playlist', {
        url: payload.url,
        headers: payload.headers ?? null,
        base: config.pixiClient.baseUrl,
      })
      return toResponse({ status: 200, headers: { 'content-type': 'application/vnd.apple.mpegurl' }, text })
    }

    // GET /m3u8?url=… — playlist straight from the scheme handler's logic.
    if (url.pathname === '/m3u8') {
      const text = await call<string>('fetch_playlist', {
        url: url.searchParams.get('url') ?? '',
        headers: null,
        base: config.pixiClient.baseUrl,
      })
      return toResponse({ status: 200, headers: { 'content-type': 'application/vnd.apple.mpegurl' }, text })
    }

    // POST /m3u8…/request — structured JSON form.
    if (url.pathname === '/request' && method === 'POST') {
      const payload = parseJson<RequestPayload>(init.body)
      const result = await call<PlainHttpResult>('http_request', {
        url: payload.url,
        method: payload.method,
        headers: withUserAgent(payload.headers),
        body: payload.body,
        contentType: payload.content_type,
        timeoutMs: null,
      })
      return toResponse(result)
    }

    // GET /request?url=… — pass-through form: init carries method/headers/body.
    if (url.pathname === '/request') {
      const result = await call<PlainHttpResult>('http_request', {
        url: url.searchParams.get('url') ?? '',
        method,
        headers: toHeaderMap(init),
        body: typeof init.body === 'string' ? init.body : null,
        contentType: null,
        timeoutMs: null,
      })
      return toResponse(result)
    }
  } catch (err) {
    if (err instanceof BridgeError || err instanceof BridgeOfflineError) throw err
    pixiHealth.getState().markOffline()
    throw new BridgeOfflineError()
  }

  throw new BridgeError('NOT_FOUND', 'Unknown bridge endpoint.', 404)
}
