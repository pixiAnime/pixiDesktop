# Repository Guidelines

## Project Overview

**PixiAnime desktop** — a Tauri 2 shell around an existing web client. The React app is
[`pixiWeb`'s source, byte-for-byte](../pixiWeb) and is **never edited**. This repo supplies only
what a browser cannot: platform code, the typed IPC surface, and the Rust services behind it
(HLS proxying, SSRF-guarded upstream HTTP, stream-to-Downloads saving).

The single most important thing to understand before touching anything:

> **`../pixiWeb` is a hard build dependency.** It must exist *and* be `pnpm install`ed. `react`,
> `react-dom` and `@types/react` are resolved out of `../pixiWeb/node_modules`
> (`vite.config.ts:57`, `tsconfig.app.json:8`) so hooks never see two React copies. If the sibling
> is missing, `pnpm check`, `pnpm build` and `pnpm dev` all fail.

## Architecture & Data Flow

```
pixiWeb React app ── module shadows ── typed Tauri commands ── Rust services
                                                      ▲
<video> / hls.js ── pixi:// scheme handler ───────────┘
```

### The three module shadows

pixiWeb keeps importing its own paths. This repo redirects exactly three of them:

| pixiWeb import | Desktop replacement | File |
| --- | --- | --- |
| `@/config` | protocol base + `isDesktop` | `src/shadow/config.ts` |
| `@/api/pixiclient` | commands instead of a localhost socket | `src/bridge/` |
| `@/components/pixiclient` | Save button, no offline dialog | `src/shadow/components-pixiclient/index.tsx` |

These are declared **twice** — as Vite aliases (`vite.config.ts:59-62`) *and* as tsconfig `paths`
(`tsconfig.app.json:10-14`). **Changing a shadow means editing both.** The typecheck covers both
trees (`include: ["src", "../pixiWeb/src"]`), so export-surface drift fails `pnpm check`. That is
the seam's whole safety mechanism — respect it.

### Communication channels

| Channel | Direction | Payload | Used for |
| --- | --- | --- | --- |
| `invoke(...)` | JS → Rust | JSON args/returns | non-streaming API calls (`http_request`, `fetch_playlist`, `download_stream`, `reveal_item`, `protocol_base`, `open_url`, `report`) |
| `app.emit(...)` | Rust → JS | unit | `stream-served` arms the Save button |
| `pixi://localhost` scheme | JS → Rust (as HTTP) | streaming bytes | `/request`, `/m3u8`, `/m3u8/segment` — the **only** path for binary/bodies too large for IPC |
| `println!` / `invoke('report')` | either → terminal | text log | the only observable output |

### Data flow — an API call

1. pixiWeb calls `bridgeRequest(...)` → `bridgeFetch('/request?...', init)` (`src/bridge/requests/index.ts`).
2. `bridgeFetch` switches on pathname/method and calls `invoke('http_request', …)`
   (`src/bridge/client/fetch.ts:122`).
3. `call<T>` normalizes: success → `markOnline()`; known error code → `BridgeError`; anything else
   → `BridgeOfflineError` (`src/bridge/client/fetch.ts:96`).
4. Rust re-validates everything — **the frontend is untrusted input, re-checked in Rust, never in
   JS** (`src-tauri/src/services/security.rs:2`). `proxy::execute` → `validate_method` /
   `validate_url` / `sanitize_request_headers` / `validate_body`, plus a per-hop `assert_public`
   SSRF check on every redirect (`src-tauri/src/services/proxy.rs`).
5. Response is size-capped, headers filtered, returned as `{status, headers, text}`.
6. `toResponse` rebuilds a real `Response` and runs pixiWeb's `classifyBridgeResponse`.

### Data flow — playback / save

`GET /m3u8?url=…` → `hls::fetch_and_rewrite` rewrites every media reference to
`<base>/m3u8/segment?url=<absolute>`, so the webview only ever speaks the app's own scheme. Each
media response calls `download::remember` and re-emits `stream-served` (self-healing listener).
`download_stream` → `save_current` writes to `app.path().download_dir()` via a `.part` file, flushed
and renamed only on success. `reveal_item` canonicalizes and refuses anything outside Downloads.

## Key Directories

| Path | Purpose |
| --- | --- |
| `src/` | Desktop TypeScript shell — 8 files, entry + bridge + shadows |
| `src/bridge/` | Tauri implementation of pixiWeb's `@/api/pixiclient` surface |
| `src/shadow/` | Module shadows: `config.ts` and `components-pixiclient/` |
| `src-tauri/src/` | Rust: `main.rs`, `protocol.rs`, `error.rs`, `commands/`, `services/` |
| `src-tauri/src/services/` | `proxy.rs`, `security.rs`, `hls.rs`, `download.rs` — where tests live |
| `src-tauri/gen/schemas/` | **Generated** by `tauri_build` — do not hand-edit |
| `src-tauri/target/`, `dist/` | Build output. There is **no `.gitignore`** in this repo; don't commit these |

## Development Commands

```sh
pnpm install                    # this repo
pnpm tauri:dev                  # dev app (Vite :1420, Rust recompiles live) ← primary loop
pnpm check                      # tsc --noEmit across BOTH trees ← the drift gate
pnpm build                      # tsc --noEmit && vite build → ./dist
cd src-tauri && cargo test      # 17 tests — HLS, security, proxy, downloads ← Rust loop
cd src-tauri && cargo clippy --all-targets -- -D warnings   # the Rust gate CI enforces
pnpm tauri:build                # release binary + installers
git tag v0.2.0 && git push origin v0.2.0                   # → CI release, all 3 platforms
```

Scoped Rust test: `cargo test services::hls` (no `--lib` — this crate is binary-only).

Never run `cargo build --release`: it produces a **dev** binary that loads `http://localhost:1420`
instead of the embedded frontend. Always go through `cargo tauri build`.

**There is no `lint` or `format` npm script, and no eslint/prettier/biome/rustfmt config.** The Rust
is hand-formatted, not rustfmt output (21 diffs at any `max_width`), so `cargo fmt --check` is
deliberately *not* in CI — reformatting the tree is a separate decision.

CI (`.github/workflows/`) enforces what already passes: `pnpm check`, `pnpm build`, and
`cargo clippy --all-targets -- -D warnings` + `cargo test` on every PR, plus a `cargo check` matrix
over Linux/macOS/Windows to catch platform-conditional compile breaks without a full bundle.

**`../pixiWeb` is checked out into `../pixiWeb` by every workflow**, because Vite and tsconfig
resolve it there and `react`/`@types/react` come from its `node_modules`. `GITHUB_TOKEN` cannot read
another private repo, so the `PIXIWEB_TOKEN` secret must exist or every job fails at checkout.

## Code Conventions & Common Patterns

**Formatting (unenforced — match it by hand):** no semicolons, single quotes, 2-space indent,
trailing commas.

**Naming:** snake_case files both trees; single-word dirs with an `index.*` barrel
(`bridge/health/index.ts`, `commands/mod.rs`). TS exports camelCase; Rust free fns snake_case,
types PascalCase. Desktop-authored TS types carry a `Bridge` prefix (`BridgeFetchInit`,
`BridgeError`) so they stay distinguishable from the verbatim re-exported originals.

**Doc comments explain *why*, and every file opens with one.** Non-obvious code carries the failure
mode it prevents; accepted shortcuts are marked `ponytail:` with an upgrade path:

```rust
/// Block loopback/private/link-local/metadata destinations. Checked for every
/// redirect hop — otherwise a public host could bounce us into the LAN.
///
/// ponytail: DNS is resolved twice (here, then again by reqwest) and a
/// TOCTOU window remains between check and connect; cache resolved addrs and
/// pin them per connection only if this ever becomes a real threat model.
pub async fn assert_public(url: &Url) -> Result<(), BridgeError> {
```
(`src-tauri/src/services/security.rs:145`)

**Error handling:** one taxonomy, shared. Rust `BridgeError { error, message, status }` with named
constructors (`invalid_url`, `blocked`, `timeout`, `from_upstream` — `src-tauri/src/error.rs:23`);
TypeScript re-exports pixiWeb's `BridgeError` / `BridgeOfflineError` / `classifyBridgeResponse`
verbatim (`src/bridge/client/errors.ts`). **Adding a code means changing both sides.**

**Async:** `async`/`await` throughout. No `.then()` chains except where a rejection handler is the
point. Rust: `async fn` returning `Result<_, BridgeError>`; the sync scheme handler spawns and hands
off to the responder (`src-tauri/src/protocol.rs:44`).

**State / DI:** none of either. State is `useState` + `useEffect`; shared state is module-level
statics (`static CURRENT: Mutex<Option<String>>` in `download.rs:27`, `OnceLock<Client>` in
`proxy.rs`). Dependencies are plain parameters (`app: &AppHandle<R>`, `incoming: &HeaderMap`).
UI phase is a discriminated union plus a `Record<Phase, string>` label map
(`src/shadow/components-pixiclient/index.tsx:18`).

**Styling:** Tailwind v4 utilities only, composed through pixiWeb's `cn()` helper
(`@/lib/utils`). No CSS files here, no `tailwind.config.*` — it is CSS-first and config-less.

**Cross-tree re-exports use deep relative paths *with* the `.ts` extension, deliberately not `@/`**
(`src/bridge/client/errors.ts:13`) so the alias mapping can never loop back into a shadow.

**Logging — the app has no console.** A GUI process's stdout is invisible from the webview. JS must
use `await invoke('report', { line })`; Rust uses `println!`. `src/main.ts:23-48` forwards
`window.onerror`, unhandled rejections, `securitypolicyviolation` and `console.error/warn` into
that channel. Run from a terminal or a blank window is undiagnosable.

**TypeScript:** `strict` is **not** enabled (`../pixiWeb/tsconfig.app.json` sets
`verbatimModuleSyntax`, `noUnusedLocals`, `noUnusedParameters`, `erasableSyntaxOnly`,
`noFallthroughCasesInSwitch` but not `strict`), so narrow by hand. Use `import type` everywhere.
Gate dev-only code with `import.meta.env.DEV` (`src/main.ts:122`).

## Important Files

| File | Why it matters |
| --- | --- |
| `vite.config.ts` | The seam. Aliases, plus three perf workarounds that must not be "cleaned up": Tailwind `source(none)` pinning (scanning `src-tauri/target` stalls cold start ~25 s), single-React pinning, `server.warmup`. |
| `tsconfig.app.json` | Mirrors the Vite aliases so drift fails `pnpm check`. Extends the sibling's config. |
| `src/main.ts` | All platform glue: error reporting, protocol-base fetch, scheme probe, `await import('@/main')` to boot React. The React tree never imports Tauri. |
| `src/bridge/client/fetch.ts` | 1:1 translation table between pixiWeb's HTTP paths and Tauri commands. The seam's most fragile contract. |
| `src/shadow/config.ts` | Lazy `baseUrl` getter — lets `src/main.ts` set the base before the app is evaluated. |
| `src-tauri/src/main.rs` | `generate_handler!` — the **complete** list of what the frontend can ask for (7 commands). |
| `src-tauri/src/error.rs` | The contract both sides code against. |
| `src-tauri/src/services/security.rs` | Trust boundary. Never move a validator into JS. |
| `src-tauri/tauri.conf.json` | CSP (`:26`) + `dangerousDisableAssetCspModification` (`:27`), devUrl 1420, frontendDist `../dist`. |
| `src-tauri/build.rs` | `rerun-if-changed=../dist` so a frontend rebuild always re-embeds. |
| `README.md` | Sole prose spec: layer map, per-platform QA status, deliberate ceilings. (Its SSRF "docs §5" pointer at `:45` is stale — the SSRF policy is §13 of `../pixiClient/docs.md`.) |

## Runtime & Tooling Preferences

- **Package manager: pnpm** (`pnpm-lock.yaml`, lockfile v9). No npm/yarn/bun. No `packageManager`
  or `engines` field, but Vite 8 requires Node `^20.19.0 || >=22.12.0`.
- **Rust:** edition 2021, tauri 2.11, rustls (no OpenSSL/SChannel glue). No `rust-toolchain.toml`.
- **`cargo tauri` must be installed separately** — `@tauri-apps/cli` is *not* an npm dependency, so
  `pnpm tauri:dev` / `tauri:build` shell out to a machine-level `cargo tauri`.
- **Linux webview deps:** `webkit2gtk-4.1`, `gtk+-3.0`.
- **Port coupling:** `tauri.conf.json` `devUrl:1420` must match `vite.config.ts` `server.port`.
- **Env vars:** no `.env` here. `VITE_*` values are inherited from pixiWeb's config module;
  `VITE_PIXICLIENT_URL` / `_TOKEN` are **inert** on desktop (base and token are forced in
  `src/shadow/config.ts:34`). To override one, put the `.env` in `pixiDesktop/`, not `pixiWeb/`.
- **CSP is shipped byte-exact on purpose.** `'unsafe-inline'` in script/style-src plus
  `dangerousDisableAssetCspModification` keeps the extension sandbox working — do not let a tool
  "fix" the CSP string.

## Testing & QA

Rust-only, entirely inline, **no separate test files and no `[dev-dependencies]`** — no mocking
library, no `tempfile`, no `wiremock`. Tests live in `#[cfg(test)] mod tests` beside the code with
`use super::*;`, so they can reach private items.

- `mod tests` — offline, fast. Best template for a new test: `src-tauri/src/services/hls.rs:110`.
- `mod network_tests` — hits the live internet and `std::env::temp_dir()`; marked "Needs network"
  (`proxy.rs:129`, `download.rs:444`). These do pass in this environment.

17 tests total (verified: `cargo test` → `17 passed`). Conventions:

- Test fn names state the invariant: `never_overwrites_a_previous_save`,
  `header_rules_match_pixiweb`, `refuses_playlists_it_cannot_just_join`.
- Async under `#[test]` via `tauri::async_runtime::block_on`, **not** `#[tokio::test]`.
- Std `assert!` / `assert_eq!` only, always with a diagnostic message.
- Filesystem tests `remove_dir_all` before *and* after; temp dirs named `pixi-dl-{pid}`.
- Snapshot testing: none. Coverage config or thresholds: none.

**No JS test framework exists.** Verifying renderer behavior means running the app
(`pnpm tauri:dev`) and reading the terminal log — which is why `report` exists.

### Deliberate ceilings — do not "fix" unasked

Recorded in `README.md:92-100`: HLS save is byte concatenation (`EXT-X-KEY` / `EXT-X-BYTERANGE` are
refused by design); no download progress, cancel, or save-as; the protocol handler buffers bodies
in RAM. Windows (`http://pixi.localhost/...`) and macOS (Gatekeeper/notarization) are **unverified**
— not known-broken. Only Linux/CachyOS is confirmed working.