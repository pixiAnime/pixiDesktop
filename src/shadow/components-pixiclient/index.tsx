/**
 * Desktop shadow of pixiWeb's `@/components/pixiclient`.
 *
 * The Android-bridge indicator and the offline dialog only make sense on the
 * web, where a separate bridge has to be reachable; on desktop the bridge is
 * compiled into the app. The header slot they used keeps its one useful job
 * here — saving the stream that is currently playing to disk.
 *
 * Hidden until Rust reports it served a stream (`stream-served`), and hidden
 * on /settings: that page renders this component again inside its About card,
 * which has nothing desktop-specific to say.
 */
import { useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { cn } from '@/lib/utils'

type Phase = 'idle' | 'saving' | 'saved' | 'error'

const LABEL: Record<Phase, string> = {
  idle: 'Save video',
  saving: 'Saving…',
  saved: 'Saved',
  error: 'Retry',
}

export function ConnectionIndicator() {
  const [served, setServed] = useState(false)
  const [phase, setPhase] = useState<Phase>('idle')
  const [file, setFile] = useState('')
  const [note, setNote] = useState('')

  // Rust re-announces `stream-served` on every media response it serves; the
  // first one arms the button. Reported once so the wiring is visible in the
  // app log — a GUI process has no console to check otherwise.
  useEffect(() => {
    let disposed = false
    let armed = false
    let unlisten: (() => void) | undefined
    void listen('stream-served', () => {
      setServed(true)
      if (armed) return
      armed = true
      void invoke('report', { line: 'stream-served: save button armed' }).catch(() => {})
    }).then(
      (fn) => {
        if (disposed) fn()
        else unlisten = fn
      },
      () => {
        // Not running inside Tauri (a plain browser), or the event permission
        // is missing — the second is worth seeing in the log, not swallowing.
        void invoke('report', { line: 'stream-served listener FAILED' }).catch(() => {})
      },
    )
    return () => {
      disposed = true
      unlisten?.()
    }
  }, [])

  // Success and failure are transient; fall back to idle on their own.
  useEffect(() => {
    if (phase !== 'saved' && phase !== 'error') return
    const timer = setTimeout(() => setPhase('idle'), 5000)
    return () => clearTimeout(timer)
  }, [phase])

  if (!served || window.location.pathname.startsWith('/settings')) return null

  async function save(): Promise<void> {
    setPhase('saving')
    try {
      setFile(await invoke<string>('download_stream'))
      setNote('')
      setPhase('saved')
    } catch (err) {
      setNote((err as { message?: string } | null)?.message ?? 'The stream could not be saved.')
      setPhase('error')
    }
  }

  async function reveal(): Promise<void> {
    try {
      await invoke('reveal_item', { path: file })
    } catch {
      // No file manager available, or the file was moved since — not worth
      // surfacing an error for an affordance this small.
    }
  }

  const busy = phase === 'saving'
  const title =
    phase === 'error'
      ? note
      : phase === 'saved'
        ? `Saved to ${file}`
        : 'Save the current stream to Downloads'

  return (
    <button
      type="button"
      aria-live="polite"
      disabled={busy}
      title={title}
      onClick={() => {
        void (phase === 'saved' ? reveal() : save())
      }}
      className={cn(
        'inline-flex h-7 items-center gap-1.5 border px-2 font-mono text-[0.65rem] tracking-wide whitespace-nowrap uppercase transition-colors outline-none',
        'focus-visible:ring-3 focus-visible:ring-ring/50 cursor-pointer',
        'border-border bg-card',
        busy
          ? 'animate-pulse border-foreground/70 text-foreground'
          : phase === 'error'
            ? 'text-muted-foreground hover:border-foreground/40 hover:text-foreground'
            : 'text-foreground hover:border-foreground/40',
      )}
    >
      <span
        aria-hidden
        className={cn(
          'size-1.5 rounded-full border',
          phase === 'error'
            ? 'border-muted-foreground/60'
            : busy
              ? 'border-foreground/70'
              : 'border-foreground bg-foreground',
        )}
      />
      <span>{LABEL[phase]}</span>
    </button>
  )
}

export function PixiOfflineDialog() {
  return null
}
