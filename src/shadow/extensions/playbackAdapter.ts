/**
 * Desktop playback adapter — shadow of pixiWeb's `player/playbackAdapter`.
 *
 * Progressive media (mp4/webm) is handed to the media element **as-is**, like
 * the web does. A `pixi://` URL is not playable there — WebKit's media loader
 * handles only http(s)/blob/file, so a scheme URL comes back
 * `MEDIA_ERR_SRC_NOT_SUPPORTED` no matter what `media-src` allows (CSP is a
 * permission check, not a transport) — and proxying would otherwise buffer a
 * whole episode in RAM before the first byte reached the player.
 *
 * HLS still goes through the scheme: hls.js (XHR + MSE, not the media loader)
 * can read `pixi://`, and that is where playlist rewriting, upstream headers
 * and CORS actually matter.
 *
 *   mp4/webm/unknown → the provider URL, untouched
 *   hls, no headers  → GET  /m3u8?url=…           (playlist rewritten to /m3u8/segment)
 *   hls + headers    → POST /m3u8 {url, headers}  → rewritten text → Blob URL
 *                      (a media element cannot set headers; Rust can)
 *
 * Blob URLs returned here must be revoked by the caller (Player does).
 *
 * The `@/extensions` barrel is what the players import, so it re-exports this
 * module over pixiWeb's own copy — see src/shadow/extensions/index.ts.
 */
import { BridgeError, BridgeOfflineError, bridgeFetch } from '@/api/pixiclient'
import { bridgePlaylistUrl } from '@/api/pixiclient/client/urls'
import { ExtensionError } from '../../../../pixiWeb/src/extensions/runtime/errors.ts'
import type { StreamSource } from '../../../../pixiWeb/src/extensions/sdk/types.ts'

export interface PlaybackTarget {
  url: string
  /** True when the caller must revokeObjectURL() on change/unmount. */
  isBlob: boolean
}

export async function preparePlayback(source: StreamSource): Promise<PlaybackTarget> {
  if (source.type !== 'hls') {
    return { url: source.url, isBlob: false }
  }

  const headers = source.headers && Object.keys(source.headers).length > 0 ? source.headers : null
  if (!headers) {
    return { url: bridgePlaylistUrl(source.url), isBlob: false }
  }

  // Protected playlist: Rust fetches it with the upstream headers and rewrites
  // every reference to the scheme, so the segments play from here too.
  let res: Response
  try {
    res = await bridgeFetch('/m3u8', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ url: source.url, headers }),
    })
  } catch (err) {
    // Message strings are the extension runtime's fixed diagnostics — they are
    // what i18n's ext-messages map keys off, so they stay byte-identical.
    if (err instanceof BridgeOfflineError) {
      throw new ExtensionError('PIXICLIENT_UNAVAILABLE', 'pixiClient is not running.')
    }
    if (err instanceof BridgeError) {
      throw new ExtensionError('PIXICLIENT_ERROR', 'The bridge could not load this stream.')
    }
    throw new ExtensionError('HTTP_ERROR', 'The stream could not be loaded.')
  }
  if (!res.ok) {
    throw new ExtensionError('HTTP_ERROR', 'The stream could not be loaded.')
  }
  const playlist = await res.text()
  if (!playlist.includes('#EXTM3U')) {
    throw new ExtensionError('HTTP_ERROR', 'The stream returned an invalid playlist.')
  }
  const blob = new Blob([playlist], { type: 'application/vnd.apple.mpegurl' })
  return { url: URL.createObjectURL(blob), isBlob: true }
}

export function revokePlayback(target: PlaybackTarget | null | undefined): void {
  if (target?.isBlob) URL.revokeObjectURL(target.url)
}
