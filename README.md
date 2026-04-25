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

The build also creates a `ghostty-terminal` WASM artifact intended to be wired to `ghostty/libghostty`.

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
  client/ghostty_terminal.zig      WASM terminal integration boundary
```

## Status

The repository has been converted to a Ziex/Zig scaffold with protocol-preserving server routes.
It also includes a dedicated WASM integration boundary for direct `ghostty/libghostty` work.
The remaining blocker is replacing the placeholder WASM terminal boundary with Ghostty's actual libghostty APIs once that dependency is vendored or exposed as a Zig package suitable for browser/WASM builds.

## Security warning

This server provides shell access. Only use it for local development and demos. Do not expose it to untrusted networks.
