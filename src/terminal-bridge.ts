// Wires browser events to PTY input (and PTY events to browser side-effects)
// for the five interop features ghostty-web exposes hooks for:
//
//   - Mouse reporting   (SGR 1006 encoding)
//   - Bracketed paste   (\e[200~ … \e[201~ wrapping)
//   - Focus events      (\e[I / \e[O on window focus/blur)
//   - Window title      (\e]0;…\a → document.title)
//   - Bell              (\a → CSS flash + Web Audio beep)
//
// Each feature is gated by a flag in `settings.ts`. attachBridge returns a
// single teardown function that disposes every listener / event subscription.

import { settings } from './settings';

interface Disposable {
  dispose(): void;
}

interface TerminalApi {
  hasMouseTracking(): boolean;
  hasBracketedPaste(): boolean;
  hasFocusEvents(): boolean;
  onTitleChange(cb: (title: string) => void): Disposable;
  onBell(cb: () => void): Disposable;
  write(data: string): void;
  renderer: { getMetrics(): { width: number; height: number } };
  /** Returning true tells the lib to skip its built-in scroll / arrow-key
   *  behavior — we use this when the program has mouse tracking enabled. */
  attachCustomWheelEventHandler(cb: (e: WheelEvent) => boolean): void;
}

export function attachBridge(
  term: TerminalApi,
  container: HTMLElement,
  send: (data: string) => void
): () => void {
  const teardown: (() => void)[] = [];
  const isTerminalSurfaceEvent = (e: MouseEvent | WheelEvent): boolean =>
    e.composedPath().some((n) => n instanceof HTMLCanvasElement);

  // ─── Bell false-positive guard ─────────────────────────────────────────────
  // ghostty-web fires its bell event whenever a write contains 0x07, which
  // also happens to be the terminator for OSC sequences (e.g. the `\e]0;…\a`
  // that bash emits on every prompt to set the window title). Patch
  // term.write to remember whether the last write also contained `\e]` — if
  // it did, the bell event we're about to receive is almost certainly the
  // OSC terminator, not a real bell.
  let lastWriteWasRealBell = false;
  const origWrite = term.write.bind(term);
  term.write = (data: string) => {
    lastWriteWasRealBell =
      typeof data === 'string' && data.includes('\x07') && !data.includes('\x1b]');
    return origWrite(data);
  };
  teardown.push(() => {
    term.write = origWrite;
  });

  // ─── Mouse ─────────────────────────────────────────────────────────────────
  if (settings.mouseEnabled) {
    const SGR_PRESS = 'M';
    const SGR_RELEASE = 'm';
    let buttonHeld = false;

    const buttonBits = (e: MouseEvent): number => {
      let b = e.button === 0 ? 0 : e.button === 1 ? 1 : e.button === 2 ? 2 : 3;
      if (e.shiftKey) b |= 4;
      if (e.altKey) b |= 8;
      if (e.ctrlKey) b |= 16;
      return b;
    };

    const cell = (e: MouseEvent) => {
      const m = term.renderer.getMetrics();
      if (!m.width || !m.height) return null;
      const r = container.getBoundingClientRect();
      const x = Math.max(1, Math.floor((e.clientX - r.left) / m.width) + 1);
      const y = Math.max(1, Math.floor((e.clientY - r.top) / m.height) + 1);
      return { x, y };
    };

    const encode = (cb: number, x: number, y: number, kind: 'M' | 'm') =>
      `\x1b[<${cb};${x};${y}${kind}`;

    const onDown = (e: MouseEvent) => {
      if (!term.hasMouseTracking()) return;
      if (!isTerminalSurfaceEvent(e)) return;
      const c = cell(e);
      if (!c) return;
      send(encode(buttonBits(e), c.x, c.y, SGR_PRESS));
      buttonHeld = true;
      e.preventDefault();
      e.stopPropagation();
    };
    const onUp = (e: MouseEvent) => {
      if (!term.hasMouseTracking()) return;
      // Allow release outside the canvas only when a drag started on-canvas.
      if (!buttonHeld && !isTerminalSurfaceEvent(e)) return;
      const c = cell(e);
      if (!c) return;
      send(encode(buttonBits(e), c.x, c.y, SGR_RELEASE));
      buttonHeld = false;
      e.preventDefault();
      e.stopPropagation();
    };
    const onMove = (e: MouseEvent) => {
      if (!term.hasMouseTracking()) return;
      if (!buttonHeld && e.buttons === 0) return;
      const c = cell(e);
      if (!c) return;
      send(encode(buttonBits(e) | 32, c.x, c.y, SGR_PRESS));
      e.preventDefault();
      e.stopPropagation();
    };
    // Wheel goes through ghostty-web's own hook so the lib's built-in scroll
    // (which doesn't know about mouse tracking) gets suppressed when the
    // program wants wheel-as-mouse. Returning true skips lib default.
    term.attachCustomWheelEventHandler((e) => {
      if (!term.hasMouseTracking()) return false;
      if (!isTerminalSurfaceEvent(e)) return false;
      const c = cell(e);
      if (!c) return false;
      const dir = e.deltaY > 0 ? 65 : 64;
      send(encode(dir, c.x, c.y, SGR_PRESS));
      return true;
    });

    const opts = { capture: true } as const;
    container.addEventListener('mousedown', onDown, opts);
    container.addEventListener('mouseup', onUp, opts);
    container.addEventListener('mousemove', onMove, opts);
    teardown.push(() => {
      container.removeEventListener('mousedown', onDown, opts);
      container.removeEventListener('mouseup', onUp, opts);
      container.removeEventListener('mousemove', onMove, opts);
      // Reset the wheel hook so a stale terminal-bridge teardown doesn't keep
      // the closure alive.
      term.attachCustomWheelEventHandler(() => false);
    });
  }

  // ─── Bracketed paste ───────────────────────────────────────────────────────
  if (settings.bracketedPasteEnabled) {
    const onPaste = (e: ClipboardEvent) => {
      const text = e.clipboardData?.getData('text/plain') ?? '';
      if (!text) return;
      e.preventDefault();
      e.stopPropagation();
      send(term.hasBracketedPaste() ? `\x1b[200~${text}\x1b[201~` : text);
    };
    container.addEventListener('paste', onPaste, { capture: true });
    teardown.push(() => container.removeEventListener('paste', onPaste, { capture: true }));
  }

  // ─── Focus events ──────────────────────────────────────────────────────────
  if (settings.focusEventsEnabled) {
    const onFocus = () => term.hasFocusEvents() && send('\x1b[I');
    const onBlur = () => term.hasFocusEvents() && send('\x1b[O');
    window.addEventListener('focus', onFocus);
    window.addEventListener('blur', onBlur);
    teardown.push(() => {
      window.removeEventListener('focus', onFocus);
      window.removeEventListener('blur', onBlur);
    });
  }

  // ─── Window title ──────────────────────────────────────────────────────────
  if (settings.setDocumentTitle) {
    const sub = term.onTitleChange((t) => {
      document.title = t || 'ghostty-web';
    });
    teardown.push(() => sub.dispose());
  }

  // ─── Bell (visual + audible + debug) ───────────────────────────────────────
  if (settings.visualBellEnabled || settings.audibleBellEnabled || settings.debugBellEnabled) {
    let audioCtx: AudioContext | null = null;
    const beep = () => {
      try {
        const Ctor = window.AudioContext ?? (window as unknown as { webkitAudioContext: typeof AudioContext }).webkitAudioContext;
        audioCtx ??= new Ctor();
        const t = audioCtx.currentTime;
        const osc = audioCtx.createOscillator();
        const gain = audioCtx.createGain();
        osc.connect(gain).connect(audioCtx.destination);
        osc.frequency.value = 800;
        gain.gain.setValueAtTime(0.08, t);
        gain.gain.exponentialRampToValueAtTime(0.001, t + 0.12);
        osc.start(t);
        osc.stop(t + 0.12);
      } catch {}
    };

    // Rate-limit: TUI apps (opencode, vim) often ring multiple bells in quick
    // succession during animations, which would otherwise be a strobe + noise
    // burst.
    let lastBellAt = 0;
    const sub = term.onBell(() => {
      if (!lastWriteWasRealBell) return; // OSC-terminator false positive
      lastWriteWasRealBell = false;
      const now = performance.now();
      if (now - lastBellAt < 250) return;
      lastBellAt = now;
      if (settings.visualBellEnabled) {
        container.classList.remove('ghostty-bell-flash');
        void container.offsetWidth; // force reflow → restart animation
        container.classList.add('ghostty-bell-flash');
      }
      if (settings.audibleBellEnabled) beep();
      if (settings.debugBellEnabled) console.log('ding');
    });
    teardown.push(() => sub.dispose());
  }

  return () => {
    for (const fn of teardown) fn();
  };
}
