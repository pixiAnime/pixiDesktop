/**
 * Desktop shell entry.
 *
 * pixiWeb's `src/main.tsx` boots the React app; this file only runs the
 * desktop-specific pre/post steps and is the single place where platform glue
 * lives (the React tree never imports Tauri directly):
 *
 *  1. ask Rust for the protocol base before the app evaluates (the config
 *     shadow reads it lazily, but doing it up front is deterministic) and
 *     probe the scheme once, so a broken transport is visible in the log,
 *  2. boot pixiWeb,
 *  3. send `target="_blank"` links to the system browser instead of letting
 *     the webview spawn a second window.
 *
 * Console output and uncaught errors are forwarded to Rust's stdout — the only
 * log a GUI process has when launched from a terminal.
 */
import { invoke } from '@tauri-apps/api/core'
import { setProtocolBase } from './shadow/config'

// The webview console is invisible from a GUI process: uncaught errors go to
// the Rust log instead, otherwise a blank window is undiagnosable.
window.addEventListener('error', (event) => {
  void invoke('report', {
    line: `error: ${event.message} @ ${event.filename}:${event.lineno}`,
  }).catch(() => {})
})
window.addEventListener('unhandledrejection', (event) => {
  void invoke('report', { line: `rejection: ${String(event.reason)}` }).catch(() => {})
})
// CSP rejections never reach window.onerror — without this a blocked fetch or
// style is an invisible blank page.
document.addEventListener('securitypolicyviolation', (event) => {
  void invoke('report', {
    line: `csp: ${event.violatedDirective} ${event.blockedURI} @ ${event.sourceFile}`,
  }).catch(() => {})
})
// console.error/warn carry the failure itself (query errors, failed loads) and
// are otherwise written to a console nobody can see from a GUI process.
for (const level of ['error', 'warn'] as const) {
  const original = console[level].bind(console)
  console[level] = (...args: unknown[]) => {
    original(...args)
    void invoke('report', {
      line: `[${level}] ${args.map((a) => (a instanceof Error ? a.stack ?? a.message : typeof a === 'object' ? JSON.stringify(a) : String(a))).join(' ').slice(0, 400)}`,
    }).catch(() => {})
  }
}

async function report(line: string): Promise<void> {
  await invoke('report', { line }).catch(() => {})
}

let base = ''
try {
  base = await invoke<string>('protocol_base')
  if (base) setProtocolBase(base)
  // One boot-time probe proves the whole transport: the scheme is registered,
  // the webview may fetch it, CORS lets us read the answer, and errors arrive
  // in pixiClient's taxonomy. No network involved (missing `url` ⇒ 400).
  const probe = await fetch(`${base}/request`)
  const body = (await probe.json()) as { error?: string }
  await report(`scheme ok: ${probe.status} ${body.error ?? ''}`)
} catch (error) {
  await report(`scheme probe failed: ${String(error)}`)
}

await import('@/main')

/**
 * Poll until the React tree renders. The mount report needs it, and so does
 * the dev transport probe below: its `stream-served` event has to land after
 * the Save button registered its listener, or the latch is invisible in the
 * log even though everything works.
 */
async function waitForMount(): Promise<number> {
  for (let attempt = 0; attempt < 200; attempt++) {
    const children = document.getElementById('root')?.childElementCount ?? 0
    if (children > 0) return children
    await new Promise((resolve) => setTimeout(resolve, 50))
  }
  return document.getElementById('root')?.childElementCount ?? -1
}

// Proves the React app actually mounted — createRoot().render() flushes on a
// scheduler tick, so this never happens synchronously.
void (async () => {
  await report(`app mounted, root children: ${await waitForMount()}`)
  // What actually rendered — a stuck splash vs. real chrome is invisible in a
  // child count, and this is the only view a headless run gets.
  const root = document.getElementById('root')
  const dom = (root?.firstElementChild?.outerHTML ?? 'EMPTY').replace(/\s+/g, ' ').slice(0, 400)
  await report(`dom: ${dom}`)
  // The route body: header rendering says nothing about whether the page's
  // data landed — this shows what Home actually painted.
  const main = (document.querySelector('main')?.innerHTML ?? 'NONE').replace(/\s+/g, ' ')
  await report(`main: ${main.slice(0, 500)}`)
  await report(`main length: ${main.length}`)
})()

document.addEventListener('click', (event) => {
  const target = event.target as Element | null
  const anchor = target?.closest?.('a[href]') as HTMLAnchorElement | null
  if (!anchor || anchor.target !== '_blank') return
  const href = anchor.href
  if (!/^https?:/i.test(href)) return
  event.preventDefault()
  void invoke('open_url', { url: href }).catch(() => {
    /* opener unavailable — never break navigation */
  })
})

/**
 * Dev-only transport check: `Range` through the scheme from the real webview.
 * Range is not a CORS-safelisted header, so this forces a preflight — the
 * OPTIONS route is covered by the same request. It deliberately targets a
 * real (tiny, Range-capable) media file: that is what makes Rust announce
 * `stream-served`, proving the Save button's arm-and-latch chain end to end
 * and leaving it a sensible target if Save is pressed before playing.
 * Never runs in a production build.
 */
if (import.meta.env.DEV && base) {
  const target = encodeURIComponent('https://mdn.github.io/shared-assets/videos/flower.mp4')
  void (async () => {
    await waitForMount()
    // useEffect commit + one IPC round-trip: the listener must be live.
    await new Promise((resolve) => setTimeout(resolve, 300))
    try {
      const ranged = await fetch(`${base}/request?url=${target}`, {
        headers: { Range: 'bytes=0-99' },
      })
      await report(
        `range: ${ranged.status} cr=${ranged.headers.get('content-range')} len=${(await ranged.text()).length}`,
      )
    } catch (error) {
      await report(`range FAILED: ${String(error)}`)
    }
  })()
}
