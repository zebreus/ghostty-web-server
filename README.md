# ghostty-web-server

A Ziex/Zig rewrite of the local Ghostty web server. The server keeps the original WebSocket envelope protocol while moving the application shell, routes, and client components to Ziex.

## Protocol

Every WebSocket frame is a JSON object.

Client to server:

- `{ "type": "input", "value": "..." }`
- `{ "type": "resize", "cols": 80, "rows": 24 }`

Server to client:

- `{ "type": "data", "value": "..." }`
- `{ "type": "ack", "cols": 80, "rows": 24 }`

## Configuration

| Env | Default | Notes |
|---|---|---|
| `PORT` | `8080` | HTTP + WebSocket port |
| `SHELL` | `/bin/bash` | Shell spawned for a session |

## Development

Install Zig 0.15.2 and the Ziex CLI, then run:

```bash
zig build dev
```

or:

```bash
zx dev
```

## Build

```bash
zig build -Doptimize=ReleaseFast
```

The build compiles Ghostty's `libghostty-vt` package for the Ziex client and also emits a small `ghostty-terminal` WASM boundary artifact.

## Architecture

```text
app/
  main.zig                         Ziex app entrypoint
  pages/page.zx                    root page
  pages/terminal_app.zx            client-rendered Ziex terminal UI shell
  pages/ws/route.zig               WebSocket endpoint
  pages/api/sessions/route.zig     session listing endpoint
  server/protocol.zig              unchanged JSON envelope protocol
  server/sessions.zig              session map, attach/takeover, scrollback
  server/pty.zig                   shell process bridge
  client/ghostty_terminal.zig      Ghostty VT terminal parser/renderer bridge
  client/frontend.zig              Ziex WebSocket client and Ghostty render loop
```

## Ghostty integration

The browser terminal path uses Ghostty directly:

- `build.zig.zon` declares `ghostty-org/ghostty` as a Zig dependency.
- `build.zig` imports Ghostty's `ghostty-vt` module for the WASM client and the focused native integration test.
- `app/client/ghostty_terminal.zig` constructs a `vt.Terminal`, keeps a persistent `vt.TerminalStream`, writes PTY chunks through Ghostty's VT parser, tracks bell/title effects, resizes with Ghostty reflow, and renders the screen via `vt.formatter.TerminalFormatter`.
- `app/client/frontend.zig` opens the preserved `/ws` protocol, persists `sessionId` in `localStorage`, sends keyboard input, and renders server `data` messages through Ghostty before updating Ziex state.

Validation:

```bash
zig build
zig build test
```

## Status

The repository has been converted to a Ziex/Zig scaffold with protocol-preserving server routes and a Ghostty-backed terminal frontend.

## ⚠️ Security warning

**This server provides shell access.** Only use it for local development and demos. Do not expose it to untrusted networks.
