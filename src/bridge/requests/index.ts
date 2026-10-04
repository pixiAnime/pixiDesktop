/**
 * Desktop twin of pixiWeb's `requests` module — same two entry points, same
 * semantics, routed through the command-backed `bridgeFetch`.
 */
import { bridgeFetch, type BridgeFetchInit, type PlainHttpResult } from '../client/fetch'
import { bridgeRequestUrl, type ProxyUrlOptions } from '../client/urls'

/** Mirrors the JSON form of pixiClient's `/request` (docs §6.6). */
export interface JsonFormPayload {
  url: string
  method?: 'GET' | 'POST' | 'PUT' | 'PATCH' | 'DELETE' | 'HEAD' | 'OPTIONS'
  /** Upstream request headers — replaces browser headers entirely. */
  headers?: Record<string, string>
  /** Request body as text (UTF-8). Binary bodies must use the query form. */
  body?: string | null
  content_type?: string
  envelope?: boolean
}

/** GET/POST/… passthrough via the query form. */
export async function bridgeRequest(
  target: string | URL,
  options: ProxyUrlOptions & BridgeFetchInit = {},
): Promise<Response> {
  const { envelope, token, ...init } = options
  const query = new URL(bridgeRequestUrl(target, { envelope, token })).searchParams.toString()
  return bridgeFetch(`/request?${query}`, init)
}

/** Structured request via the JSON form — for browser-forbidden headers. */
export async function bridgeRequestJson(payload: JsonFormPayload): Promise<Response> {
  return bridgeFetch('/request', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({
      method: 'GET',
      headers: {},
      body: null,
      envelope: false,
      ...payload,
    }),
  })
}

/** Convenience: JSON form GET returning parsed data. */
export async function bridgeGetJson<T>(target: string | URL): Promise<T> {
  const res = await bridgeRequest(target, { headers: { Accept: 'application/json' } })
  if (!res.ok) throw new Error(`Upstream request failed (HTTP ${res.status})`)
  return (await res.json()) as T
}

export type { PlainHttpResult }
