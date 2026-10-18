<div align="center">

<img src="src-tauri/icons/icon.png" width="96" alt="PixiAnime logo" />

# PixiAnime Desktop

**A native desktop client for PixiAnime — built with Tauri 2.**

[![CI](https://github.com/pixiAnime/pixiDesktop/actions/workflows/ci.yml/badge.svg)](https://github.com/pixiAnime/pixiDesktop/actions/workflows/ci.yml)
[![Release](https://github.com/pixiAnime/pixiDesktop/actions/workflows/release.yml/badge.svg)](https://github.com/pixiAnime/pixiDesktop/actions/workflows/release.yml)

`Tauri 2` `Rust 2021` `Vite 8` `TypeScript 6` `Tailwind 4` `pnpm`

</div>

---

## What is this?

PixiAnime Desktop is the **desktop shell** for the PixiAnime web client. The user interface is the
web client's own React source, used unmodified — this repository supplies everything a browser
cannot:

- 🛡️ **SSRF-guarded upstream HTTP** — the renderer has no network access at all; Rust re-validates
  every request against private-range, link-local and metadata targets, on *every* redirect hop.
- 📺 **HLS rewriting in Rust** — M3U8 playlists and segments are rewritten to an app-private URI
  scheme so the player gets the upstream headers and a CORS-clean origin. Progressive media
  (mp4/webm) is fetched by the media element straight from the provider, as on the web, so seeking
  is native and a 1 GB episode never lands in RAM.
- 💾 **Save-to-Downloads, in Rust** — the proxy can persist the stream currently playing to
  `~/Downloads` with collision-proof naming and atomic writes. Currently unreachable from the UI:
  see *Known limitations*.
- 🔒 **Hardened CSP** — a single explicit policy that keeps the extension sandbox functional.

> **🚧 Early software — v0.1.0.** Verified on Linux; Windows and macOS builds are produced by CI
> but have not been hand-QA'd. See [Known limitations](#known-limitations).

## Architecture

The shell swaps exactly four modules out of the web client. Everything else is the original source.

```mermaid
graph LR
    A["pixiWeb React app<br/><i>(untouched, ../pixiWeb)</i>"] --> B["Module shadows<br/>@/config · @/api/pixiclient · @/components/pixiclient · @/extensions"]
    B --> C["Typed Tauri commands<br/><i>invoke()</i>"]
    C --> D["Rust services<br/>proxy · security · hls · download"]
    E["&lt;video&gt; / hls.js"] --> F["pixi:// scheme handler<br/><i>/request · /m3u8 · /m3u8/segment</i>"]
    F --> D
    E -->|"mp4/webm: direct"| G["Provider CDN"]
    style A fill:#1f2937,stroke:#4b5563,color:#f9fafb
    style D fill:#14532d,stroke:#16a34a,color:#f9fafb
    style F fill:#1e3a8a,stroke:#3b82f6,color:#f9fafb
```

| pixiWeb import | Desktop replacement | What changes |
| --- | --- | --- |
| `@/config` | `src/shadow/config.ts` | protocol base + `isDesktop` |
| `@/api/pixiclient` | `src/bridge/` | localhost socket → Tauri commands |
| `@/components/pixiclient` | `src/shadow/components-pixiclient/` | connection status + offline dialog → nothing |
| `@/extensions` | `src/shadow/extensions/` | provider URL → the app's own scheme |

Because the aliases and the TypeScript `paths` point at the same files, `pnpm check` type-checks the
web client's own imports against the desktop shadows — **export-surface drift fails the build.**

## Quick start

`pixiDesktop` requires its sibling web client at `../pixiWeb`. Clone them side by side:

```sh
git clone https://github.com/pixiAnime/pixiDesktop.git
git clone https://github.com/pixiAnime/pixiWeb.git ../pixiWeb

cd pixiDesktop
pnpm install
pnpm install --dir ../pixiWeb     # react + types resolve from here
pnpm tauri:dev
```

> The sibling checkout is a **hard build dependency** — `react`, `react-dom` and `@types/react` are
> resolved out of `../pixiWeb/node_modules` so hooks never see two copies of React.

### Prerequisites

| | Requirement |
| --- | --- |
| Node | `^20.19` or `>=22.12` (Vite 8) |
| Package manager | **pnpm** — `corepack enable pnpm` |
| Rust | stable toolchain, edition 2021 |
| Linux | `webkit2gtk-4.1`, `gtk+-3.0`, `libssl`, `libxdo` |

On Linux the webview ships with a WebKitGPU fallback for NVIDIA/Wayland hosts:
`main()` sets `WEBKIT_DISABLE_DMABUF_RENDERER` and `__NV_DISABLE_EXPLICIT_SYNC`
unless already present in the environment — the DMABUF renderer otherwise
flickers on scroll and freezes the webview in video playback on proprietary
NVIDIA (tauri-apps/tauri#9394, WebKit bug 260654). Set either variable yourself
to override.

## Development

```sh
pnpm tauri:dev              # run the app (Vite :1420 + live Rust)
pnpm dev                    # frontend only, in a browser
pnpm check                  # typecheck both trees — the drift gate
pnpm build                  # typecheck + bundle → ./dist
cd src-tauri && cargo test  # 17 Rust tests
```

| Script | Does |
| --- | --- |
| `pnpm dev` | Vite dev server only — not the app |
| `pnpm check` | `tsc --noEmit` across **both** source trees |
| `pnpm build` | `tsc --noEmit` then `vite build` → `dist/` |
| `pnpm tauri:dev` | The real desktop app |
| `pnpm tauri:build` | Release binary + installers |

There is deliberately no `lint` or `format` script — style is carried by the surrounding code.

<details>
<summary>Debugging a blank window</summary>

A GUI process has no console. JS errors, unhandled rejections, CSP violations and
`console.error`/`warn` are all forwarded to Rust's stdout through the `report` command, and the
scheme handler logs with `println!`.

**Run the app from a terminal** — otherwise a blank window is undiagnosable.
</details>

## Releases

Push a tag and CI builds all three platforms:

```sh
git tag v0.1.0 && git push origin v0.1.0
```

A draft GitHub release is created with the installers attached:

| Platform | Artifacts |
| --- | --- |
| Linux | `.deb`, `.rpm`, `.AppImage` |
| Windows | `.msi`, `.exe` (NSIS setup) |
| macOS | `.dmg` + `.app` — Apple Silicon and Intel (x86_64) |

Bundle targets are Tauri 2's defaults — `bundle.targets` is unset in `tauri.conf.json`. GitHub
retired its Intel macOS runners, so the x86_64 slice is cross-compiled from an Apple Silicon runner.

Apple code-signing and notarization activate automatically if the standard `APPLE_*` secrets are
configured in repository settings; unsigned builds are produced otherwise. Windows and macOS
bundling needs no extra setup.

### No setup needed

Both workflows check [`pixiAnime/pixiWeb`](https://github.com/pixiAnime/pixiWeb) out into
`../pixiWeb` themselves, so there is nothing to configure — no tokens, no secrets. Apple signing
and notarization are the only optional secrets, and their absence is handled gracefully.

## Repository layout

```text
pixiDesktop/
├── src/                     # TypeScript desktop shell
│   ├── main.ts              # entry: logging, protocol base, boots React
│   ├── bridge/              # Tauri impl. of @/api/pixiclient
│   │   ├── client/          # fetch, urls, errors
│   │   ├── health/          # connectivity probe (always online in-process)
│   │   └── requests/        # bridgeRequest helpers
│   └── shadow/              # @/config, @/components/pixiclient, @/extensions
└── src-tauri/
    ├── src/
    │   ├── main.rs          # command registration + scheme registration
    │   ├── protocol.rs      # pixi:// handler — streaming transport
    │   ├── error.rs         # BridgeError — the shared error taxonomy
    │   ├── commands/        # typed IPC surface
    │   └── services/        # proxy · security · hls · download (+ their tests)
    ├── capabilities/        # Tauri v2 ACL
    └── tauri.conf.json      # bundle, CSP, window
```

Agent-facing conventions live in [`AGENTS.md`](./AGENTS.md).

## Security model

The renderer is treated as **untrusted input**. It has no direct network or filesystem access; it
can only invoke the seven registered commands, and every one re-runs the web client's own
validators in Rust.

- **URL validation** — scheme allow-list, length cap, CRLF-injection rejection.
- **SSRF guard** — DNS-resolved addresses are checked against loopback, private, CGNAT,
  link-local and cloud-metadata ranges, **on every redirect hop**.
- **Header hygiene** — hop-by-hop and transport-owned headers are stripped before forwarding.
- **Size caps** — request bodies, header counts, and response bodies all bounded.
- **File access** — writes go to `~/Downloads` only, via a `.part` file renamed atomically on
  success; `reveal_item` canonicalizes and refuses anything outside Downloads.

## Known limitations

These are deliberate, documented ceilings — not open bugs:

- **No Save affordance in the UI.** The header slot pixiWeb fills with its `● pixiClient Connected`
  badge renders nothing here — the bridge is compiled into the app, so there is no connection to
  announce. The Rust save service (`download_stream` / `reveal_item`) and its tests are intact but
  have no caller; it needs a new home (the player) before Stream → Downloads is a feature again.
- **HLS saving is byte concatenation.** Init segment + fragments (`.mp4`) or whole segments (`.ts`).
  `EXT-X-KEY` (AES-128) and `EXT-X-BYTERANGE` playlists are refused with a clear error rather than
  silently mis-saved — they need decryption and demuxing.
- **No download progress, cancel, or save-as** in the service itself; it writes to `~/Downloads` with
  unique names and a `.part` file until complete.
- **The protocol handler buffers bodies in RAM.** Fine for playlists and segments, which are all it
  serves now — progressive media goes straight from the media element to the CDN. The upgrade path
  for anything larger is a loopback HTTP server.
- **Two Settings panels describe web-only transport.** "Request servers" stores external bridge
  endpoints, and the Android banner sells a mobile app — a desktop build has neither: the bridge is
  this process. The server panel's *Test* button is an ordinary probe, so it reports truthfully, but
  no request is ever routed through a listed server. Segment requests also replay only the headers
  the webview itself sends, so a provider that gates segments on a custom `Referer` fails soft.
- **Windows and macOS are untested by hand.** The Windows `http://pixi.localhost/…` URL shape and
  macOS Gatekeeper notarization have not been exercised.

---

<div align="center">
<sub>Built with Tauri, Rust and a lot of care for the streaming edge cases.</sub>
</div>