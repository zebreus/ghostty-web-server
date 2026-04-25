const encoder = new TextEncoder();
const decoder = new TextDecoder();

const settings = { minCols: 20, minRows: 4, cellWidth: 8, cellHeight: 17 };

function uuid() {
  const b = crypto.getRandomValues(new Uint8Array(16));
  b[6] = (b[6] & 0x0f) | 0x40;
  b[8] = (b[8] & 0x3f) | 0x80;
  const h = [...b].map((x) => x.toString(16).padStart(2, '0')).join('');
  return `${h.slice(0, 8)}-${h.slice(8, 12)}-${h.slice(12, 16)}-${h.slice(16, 20)}-${h.slice(20)}`;
}

function sessionId() {
  let id = sessionStorage.getItem('ghosttySessionId');
  if (!id) {
    id = uuid();
    sessionStorage.setItem('ghosttySessionId', id);
  }
  return id;
}

function consumeTakeFlag() {
  const take = sessionStorage.getItem('ghosttyTake') === '1';
  if (take) sessionStorage.removeItem('ghosttyTake');
  return take;
}

class TerminalView {
  constructor(root) {
    this.root = root;
    this.pre = document.createElement('pre');
    this.pre.className = 'terminal-screen';
    this.pre.tabIndex = 0;
    this.buffer = '';
    this.maxBuffer = 256000;
    root.replaceChildren(this.pre);
  }

  write(data) {
    // Strip OSC (`ESC ] ... BEL` / `ESC ] ... ESC \`) and CSI
    // (`ESC [ ... final-byte`) control sequences before appending to the
    // minimal text renderer. The libghostty-backed renderer will consume these
    // natively once the full terminal grid integration lands.
    this.buffer += data.replace(/\x1b\][^\x07]*(?:\x07|\x1b\\)/g, '').replace(/\x1b\[[0-?]*[ -/]*[@-~]/g, '');
    if (this.buffer.length > this.maxBuffer) this.buffer = this.buffer.slice(-192000);
    this.pre.textContent = this.buffer;
    this.pre.scrollTop = this.pre.scrollHeight;
  }

  bell() {
    this.pre.classList.remove('ghostty-bell-flash');
    void this.pre.offsetWidth;
    this.pre.classList.add('ghostty-bell-flash');
  }

  size() {
    return {
      cols: Math.max(settings.minCols, Math.floor(this.root.clientWidth / settings.cellWidth)),
      rows: Math.max(settings.minRows, Math.floor(this.root.clientHeight / settings.cellHeight)),
    };
  }
}

async function loadWasm(view) {
  let instance;
  const imports = {
    env: {
      js_render_text(ptr, len) {
        const bytes = new Uint8Array(instance.exports.memory.buffer, ptr, len);
        view.write(decoder.decode(bytes));
      },
      js_set_title(ptr, len) {
        const bytes = new Uint8Array(instance.exports.memory.buffer, ptr, len);
        document.title = decoder.decode(bytes) || 'ghostty-web';
      },
      js_bell() { view.bell(); },
    },
  };
  try {
    const result = await WebAssembly.instantiateStreaming(fetch('/client.wasm'), imports);
    instance = result.instance;
  } catch {
    const result = await WebAssembly.instantiate(await fetch('/client.wasm').then((r) => r.arrayBuffer()), imports);
    instance = result.instance;
  }
  return instance;
}

async function main() {
  const root = document.getElementById('terminal');
  const status = document.getElementById('status');
  const view = new TerminalView(root);
  let wasm = null;
  try { wasm = await loadWasm(view); } catch (err) { console.warn('client wasm unavailable, using JS renderer', err); }

  let ws;
  let take = consumeTakeFlag();
  let size = view.size();
  wasm?.exports.terminal_init?.(size.cols, size.rows);

  const send = (msg) => {
    if (ws?.readyState === WebSocket.OPEN) ws.send(JSON.stringify(msg));
  };

  const feed = (text) => {
    if (!wasm?.exports.alloc || !wasm?.exports.terminal_feed) {
      view.write(text);
      return;
    }
    const bytes = encoder.encode(text);
    const ptr = wasm.exports.alloc(bytes.length);
    new Uint8Array(wasm.exports.memory.buffer, ptr, bytes.length).set(bytes);
    wasm.exports.terminal_feed(ptr, bytes.length);
    wasm.exports.free?.(ptr, bytes.length);
  };

  const connect = () => {
    status.textContent = 'Connecting…';
    const proto = location.protocol === 'https:' ? 'wss:' : 'ws:';
    const params = new URLSearchParams({ sessionId: sessionId(), cols: String(size.cols), rows: String(size.rows) });
    if (take) { params.set('take', '1'); take = false; }
    ws = new WebSocket(`${proto}//${location.host}/ws?${params}`);
    ws.onopen = () => { status.textContent = ''; };
    ws.onmessage = (event) => {
      if (typeof event.data !== 'string') return;
      try {
        const msg = JSON.parse(event.data);
        if (msg.type === 'data' && typeof msg.value === 'string') feed(msg.value);
        if (msg.type === 'ack' && Number.isFinite(msg.cols) && Number.isFinite(msg.rows)) {
          size = { cols: msg.cols, rows: msg.rows };
          wasm?.exports.terminal_resize?.(size.cols, size.rows);
        }
      } catch {}
    };
    ws.onclose = (event) => {
      if (event.code === 4002) {
        status.innerHTML = '<button id="take-session">Take session</button>';
        document.getElementById('take-session').onclick = () => {
          sessionStorage.setItem('ghosttyTake', '1');
          location.reload();
        };
      } else {
        status.textContent = 'Disconnected. Reconnecting…';
        setTimeout(connect, 2000);
      }
    };
  };

  view.pre.addEventListener('keydown', (event) => {
    // Reserve Ctrl+K for the command palette shortcut planned for the Ziex UI.
    if (event.ctrlKey && event.key === 'k') return;
    if (event.key.length === 1) {
      send({ type: 'input', value: event.key });
      event.preventDefault();
    } else if (event.key === 'Enter') send({ type: 'input', value: '\r' });
    else if (event.key === 'Backspace') send({ type: 'input', value: '\x7f' });
    else if (event.key === 'Tab') send({ type: 'input', value: '\t' });
    else if (event.key === 'ArrowUp') send({ type: 'input', value: '\x1b[A' });
    else if (event.key === 'ArrowDown') send({ type: 'input', value: '\x1b[B' });
    else if (event.key === 'ArrowRight') send({ type: 'input', value: '\x1b[C' });
    else if (event.key === 'ArrowLeft') send({ type: 'input', value: '\x1b[D' });
  });
  view.pre.addEventListener('paste', (event) => {
    const text = event.clipboardData?.getData('text/plain') ?? '';
    if (text) send({ type: 'input', value: text });
    event.preventDefault();
  });
  new ResizeObserver(() => {
    const next = view.size();
    if (next.cols !== size.cols || next.rows !== size.rows) {
      size = next;
      send({ type: 'resize', cols: size.cols, rows: size.rows });
    }
  }).observe(root);
  connect();
  view.pre.focus();
}

main().catch((err) => {
  document.body.textContent = `Failed to start terminal: ${err?.message ?? err}`;
});
