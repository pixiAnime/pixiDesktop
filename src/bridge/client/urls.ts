/**
 * URL builders — reusing pixiWeb's implementation unchanged.
 *
 * It builds every URL off `config.pixiClient.baseUrl`, which the desktop config
 * shadow points at the app's own protocol base (`pixi://localhost`, or
 * `http://pixi.localhost` on Windows) — so `/request`, `/m3u8` and
 * `/m3u8/segment` land in Rust's scheme handler instead of a localhost socket.
 */
export {
  bridgePlaylistUrl,
  bridgeRequestUrl,
  bridgeSegmentUrl,
  bridgeTargetOf,
  type ProxyUrlOptions,
} from '../../../../pixiWeb/src/api/pixiclient/client/urls.ts'
