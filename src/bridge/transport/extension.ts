/**
 * Desktop twin of pixiWeb's browser-extension transport.
 *
 * There is no extension to talk to: the privileged request path *is* this app
 * (Rust commands + the scheme handler), so the transport module exists only so
 * pixiWeb's own sources keep the same import surface. Every name here is a
 * deliberate no-op or a constant, and they are only reached through the `@/`
 * alias — pixiWeb's runtime/health modules import their own version directly.
 */

/**
 * The web asks the extension to attach `headers` (Referer, Cookie, …) to a
 * host's subresource requests through declarativeNetRequest. Nothing equivalent
 * exists in a webview: segment and key requests replay only the browser's own
 * headers (see `protocol::forwarded`), which is enough for the streams that
 * play today.
 *
 * ponytail: a playlist fetched with upstream headers gets rewritten segments
 * that carry no headers of their own — providers that gate *segments* on a
 * Referer fail soft to a player error. Upgrade path: a per-host header rule set
 * in Rust, consulted by `forwarded`, fed from here.
 */
export function allowHostHeaders(hosts: string[], headers: Record<string, string>): void {
  void hosts
  void headers
}

/** The shell ships for desktop platforms only — never the Android upsell banner. */
export function isAndroid(): boolean {
  return false
}
