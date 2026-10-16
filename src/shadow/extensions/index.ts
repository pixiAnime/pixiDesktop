/**
 * Desktop shadow of pixiWeb's `@/extensions` barrel.
 *
 * Everything pixiWeb's extension system offers is reused verbatim — the SDK,
 * the sandboxed runtime, the provider aggregators, the subtitles pipeline. Only
 * playback preparation differs: progressive media keeps the provider URL (as on
 * the web), HLS is routed through the scheme handler — see ./playbackAdapter.ts.
 *
 * The players import `preparePlayback` from this barrel, which pixiWeb's copy
 * re-exports from its own adapter by *relative* path — so shadowing the adapter
 * module alone would never be reached. `@/extensions/…` subpaths (subtitles,
 * providers, runtime, sdk) stay pixiWeb's, in both the Vite aliases and the
 * tsconfig paths.
 */
export * from '../../../../pixiWeb/src/extensions/index.ts'
export { preparePlayback, revokePlayback } from './playbackAdapter'
export type { PlaybackTarget } from './playbackAdapter'
