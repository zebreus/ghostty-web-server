import { useEffect, useRef } from 'preact/hooks';
import { setStatus } from '../status';
import { switchSession } from '../session-commands';
import { settings } from '../settings';
import { attachBridge } from '../terminal-bridge';

type GhosttyModule = {
  init: () => Promise<unknown>;
  Terminal: new (opts: Record<string, unknown>) => GhosttyTerminal;
};

interface GhosttyTerminal {
  cols: number;
  rows: number;
  renderer: { getMetrics(): { width: number; height: number } };
  open(el: HTMLElement): Promise<void>;
  resize(cols: number, rows: number): void;
  write(data: string): void;
  onData(cb: (data: string) => void): void;
  onResize(cb: (size: { cols: number; rows: number }) => void): void;
  hasMouseTracking(): boolean;
  hasBracketedPaste(): boolean;
  hasFocusEvents(): boolean;
  onTitleChange(cb: (title: string) => void): { dispose(): void };
  onBell(cb: () => void): { dispose(): void };
  dispose?(): void;
  /** Current viewport scroll offset (0 = bottom, >0 = scrolled up into history). */
  viewportY: number;
  /** Number of lines in the scrollback buffer (history above the active screen). */
  getScrollbackLength(): number;
}

// RFC4122 v4 via getRandomValues, which (unlike crypto.randomUUID) works on
// plain-HTTP non-localhost origins too.
function uuid(): string {
  const b = crypto.getRandomValues(new Uint8Array(16));
  b[6] = (b[6] & 0x0f) | 0x40;
  b[8] = (b[8] & 0x3f) | 0x80;
  const h = [...b].map((x) => x.toString(16).padStart(2, '0')).join('');
  return `${h.slice(0, 8)}-${h.slice(8, 12)}-${h.slice(12, 16)}-${h.slice(16, 20)}-${h.slice(20)}`;
}

function getSessionId(): string {
  let id = sessionStorage.getItem('ghosttySessionId');
  if (!id) {
    id = uuid();
    sessionStorage.setItem('ghosttySessionId', id);
  }
  return id;
}

function consumeTakeFlag(): boolean {
  const taken = sessionStorage.getItem('ghosttyTake') === '1';
  if (taken) sessionStorage.removeItem('ghosttyTake');
  return taken;
}

export function TerminalIsland() {
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    let cancelled = false;
    let ws: WebSocket | undefined;
    let term: GhosttyTerminal | undefined;
    let take = consumeTakeFlag();
    let reconnectTimer: ReturnType<typeof setInterval> | undefined;
    let ro: ResizeObserver | undefined;
    let detachBridge: (() => void) | undefined;

    (async () => {
      const url = `${location.origin}/dist/ghostty-web.js`;
      const { init, Terminal } = (await import(/* @vite-ignore */ url)) as GhosttyModule;
      if (cancelled) return;

      // Silence ghostty-web's noisy `[ghostty-vt]` log output (mostly OSC
      // warnings for color/cursor queries, hyperlinks, etc. that the WASM
      // parser doesn't fully implement — informational, doesn't break
      // anything). The lib calls console.log with `'[ghostty-vt]'` as the
      // first arg.
      const origLog = console.log;
      console.log = (...args: unknown[]) => {
        if (args[0] === '[ghostty-vt]') return;
        origLog(...args);
      };

      await init();
      term = new Terminal({
        cols: 80,
        rows: 24,
        fontFamily: 'JetBrains Mono, Menlo, Monaco, monospace',
        fontSize: 14,
        theme: { background: '#1e1e1e', foreground: '#d4d4d4' },
      });
      await term.open(ref.current!);

      // Wire-protocol envelope. Every WS frame in either direction is JSON.
      type ClientMsg = { type: 'input'; value: string } | { type: 'resize'; cols: number; rows: number };
      const sendMsg = (msg: ClientMsg) => {
        if (ws && ws.readyState === WebSocket.OPEN) ws.send(JSON.stringify(msg));
      };

      // Mouse, bracketed paste, focus, title, bell — all in one place.
      detachBridge = attachBridge(term, ref.current!, (d) =>
        sendMsg({ type: 'input', value: d })
      );

      // ACK-based resize. fit() sends the desired size; term.resize is only
      // applied when the server has confirmed it. One in flight at a time so
      // bash never sees a SIGWINCH storm.
      let inFlightResize = false;
      let desiredSize: { cols: number; rows: number } | null = null;
      let settleTimer: ReturnType<typeof setTimeout> | null = null;

      const sendNextResize = () => {
        if (!desiredSize) return;
        inFlightResize = true;
        sendMsg({ type: 'resize', cols: desiredSize.cols, rows: desiredSize.rows });
      };

      const onAck = (cols: number, rows: number) => {
        inFlightResize = false;
        if (term && (cols !== term.cols || rows !== term.rows)) term.resize(cols, rows);
        if (desiredSize && (desiredSize.cols !== cols || desiredSize.rows !== rows)) {
          sendNextResize();
        } else {
          desiredSize = null;
        }
      };

      const fit = () => {
        if (!term || !ref.current) return;
        const m = term.renderer.getMetrics();
        if (!m.width || !m.height) return;
        const cols = Math.max(settings.minCols, Math.floor(ref.current.clientWidth / m.width));
        const rows = Math.max(settings.minRows, Math.floor(ref.current.clientHeight / m.height));
        if (cols === term.cols && rows === term.rows && !desiredSize) return;
        desiredSize = { cols, rows };
        if (!inFlightResize) sendNextResize();
        if (settings.resizeAutoRedrawMs > 0) {
          if (settleTimer) clearTimeout(settleTimer);
          settleTimer = setTimeout(() => {
            settleTimer = null;
            sendMsg({ type: 'input', value: '\x0c' });
          }, settings.resizeAutoRedrawMs);
        }
      };
      ro = new ResizeObserver(fit);
      ro.observe(ref.current!);

      const sessionId = getSessionId();

      const connect = () => {
        if (cancelled) return;
        setStatus({ kind: 'connecting' });
        const proto = location.protocol === 'https:' ? 'wss:' : 'ws:';
        const params = new URLSearchParams({
          sessionId,
          cols: String(term!.cols),
          rows: String(term!.rows),
        });
        if (take) {
          params.set('take', '1');
          take = false;
        }
        ws = new WebSocket(`${proto}//${location.host}/ws?${params}`);
        ws.onopen = () => {
          setStatus({ kind: 'connected' });
          // Re-sync after (re)connect: resend the current desired size if any.
          inFlightResize = false;
          if (!desiredSize) fit();
          else sendNextResize();
        };
        ws.onmessage = (e) => {
          if (typeof e.data !== 'string') return;
          try {
            const m = JSON.parse(e.data);
            if (m.type === 'data' && typeof m.value === 'string') {
              // ghostty-web's write() unconditionally scrolls to bottom.
              // Preserve the user's scroll position so they can read history
              // while output is still arriving.
              const savedY = term!.viewportY;
              const savedLen = savedY > 0 ? term!.getScrollbackLength() : 0;
              term!.write(m.value);
              if (savedY > 0) {
                // Adjust for any new scrollback lines so the viewport stays
                // at the same absolute position in the buffer.
                const delta = term!.getScrollbackLength() - savedLen;
                term!.viewportY = savedY + delta;
              }
            } else if (
              m.type === 'ack' &&
              typeof m.cols === 'number' &&
              typeof m.rows === 'number'
            ) {
              onAck(m.cols, m.rows);
            }
          } catch {}
        };
        ws.onclose = (e) => {
          if (cancelled) return;
          if (e.code === 4002) {
            setStatus({
              kind: 'busy',
              onTake: () => switchSession(sessionId, true),
            });
            return;
          }
          let n = 2;
          setStatus({ kind: 'reconnecting', in: n });
          reconnectTimer = setInterval(() => {
            n -= 1;
            if (n <= 0) {
              clearInterval(reconnectTimer);
              connect();
            } else {
              setStatus({ kind: 'reconnecting', in: n });
            }
          }, 1000);
        };
      };
      connect();

      term.onData((d) => sendMsg({ type: 'input', value: d }));
    })();

    return () => {
      cancelled = true;
      if (reconnectTimer) clearInterval(reconnectTimer);
      detachBridge?.();
      ro?.disconnect();
      ws?.close();
      term?.dispose?.();
    };
  }, []);

  return <div ref={ref} id="terminal" />;
}
