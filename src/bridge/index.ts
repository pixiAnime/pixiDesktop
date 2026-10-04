/**
 * Desktop `@/api/pixiclient` — the same public surface as pixiWeb's barrel,
 * backed by Tauri instead of a localhost socket.
 *
 * Nothing above this layer changes: playbackAdapter, extension http, subtitle
 * tracks, ExtensionLoader and jikan keep importing the same names.
 */
export { bridgeFetch, type BridgeFetchInit, type PlainHttpResult } from './client/fetch'
export {
  bridgePlaylistUrl,
  bridgeRequestUrl,
  bridgeSegmentUrl,
  bridgeTargetOf,
  type ProxyUrlOptions,
} from './client/urls'
export {
  BridgeError,
  BridgeOfflineError,
  classifyBridgeResponse,
  isBridgeErrorCode,
  type BridgeErrorCode,
} from './client/errors'
export {
  pixiHealth,
  probePixiClient,
  waitForPixiClient,
  ONLINE_INTERVAL_MS,
  OFFLINE_INTERVAL_MS,
  PROBE_TIMEOUT_MS,
  type PixiStatus,
} from './health'
