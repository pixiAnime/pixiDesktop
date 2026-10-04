/**
 * Desktop bridge health — pixiWeb's store with a desktop probe.
 *
 * On web the bridge is another process, so the store polls it. On desktop the
 * bridge *is* this process: if Rust is answering IPC, the app is alive, and if
 * it isn't, the app itself is gone — so the probe is a constant "online".
 * Everything else (store shape, intervals, LNA helpers) is reused unchanged,
 * which keeps `usePixiHealth`, `usePixiGuard` and the guard-driven flows in
 * pixiWeb working as-is.
 */
import { pixiHealth } from '../../../../pixiWeb/src/api/pixiclient/health/index.ts'
import type { PixiStatus } from '../../../../pixiWeb/src/api/pixiclient/health/index.ts'

export * from '../../../../pixiWeb/src/api/pixiclient/health/index.ts'
export type { PixiStatus } from '../../../../pixiWeb/src/api/pixiclient/health/index.ts'

// Skip the "connecting" flash on first paint — there is nothing to connect to.
pixiHealth.getState().markOnline()

/** Desktop probe: the bridge lives in-process, so the answer is always online. */
export async function probePixiClient(): Promise<PixiStatus> {
  pixiHealth.getState().markOnline()
  return 'online'
}

/** Desktop retry: never has to wait for a separate process. */
export async function waitForPixiClient(
  { attempts = 5, delayMs = 400 }: { attempts?: number; delayMs?: number } = {},
): Promise<boolean> {
  void attempts
  void delayMs
  pixiHealth.getState().markOnline()
  return true
}
