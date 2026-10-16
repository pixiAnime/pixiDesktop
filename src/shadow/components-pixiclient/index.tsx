/**
 * Desktop shadow of pixiWeb's `@/components/pixiclient` — all three members
 * render nothing.
 *
 * Every one of them exists to talk to a bridge that lives *outside* the page:
 * a socket on the web, an Android app, a browser extension. On desktop the
 * bridge is compiled into the process, so there is no connection to announce
 * and nothing to reconnect: the header slot the web fills with
 * "● pixiClient Connected" (and `Settings` re-renders inside its About card)
 * stays empty, which is the honest reading of a client that is always there.
 */

/** Web: `● pixiClient Connected|Connecting…|Offline` + the offline dialog it opens. */
export function ConnectionIndicator() {
  return null
}

/** Web: modal offering the alternate transports when the socket is down. */
export function PixiOfflineDialog() {
  return null
}

/** Android-app upsell (web, mobile browsers only) — nothing to show in a desktop shell. */
export function AndroidAppBanner() {
  return null
}
