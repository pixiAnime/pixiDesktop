/**
 * Desktop twin of pixiWeb's `servers/protocol` (Settings → Request servers).
 *
 * The web keeps a list of pixiClient-compatible endpoints so a visitor without
 * the browser extension still has a bridge, and routes requests through them.
 * This shell needs none of that: the bridge is in this process and is never
 * swapped — a server entered in Settings is stored by pixiWeb's own store but
 * no desktop request goes through it.
 *
 * The module exists so pixiWeb's Settings page type-checks and runs unchanged,
 * and it is deliberately pixiWeb's implementation rather than a desktop twin:
 * the probe is an ordinary `fetch` of `<server>/`, which answers truthfully for
 * a real pixiClient server reached over https. It cannot reach `http://` LAN or
 * loopback servers (CSP `connect-src`, and Rust's SSRF guard behind it) — which
 * is the same shape as the rest of this app, where only the scheme handler and
 * the commands may open a socket.
 *
 * ponytail: hide the Request servers panel on desktop if a list that does not
 * route ever confuses someone; that needs a pixiWeb-side flag.
 */
export * from '../../../../pixiWeb/src/api/pixiclient/servers/protocol.ts'
