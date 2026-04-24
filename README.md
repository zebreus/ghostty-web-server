# ghostty-web-server

A local web server that opens a real shell session in your browser, rendered with the
[ghostty-web](https://github.com/coder/ghostty-web) terminal emulator. Built on
[Bun](https://bun.com) + [Elysia](https://elysiajs.com) with a tiny
[Preact](https://preactjs.com) island for the terminal. POSIX only.

## Quick start

### Single binary (no runtime needed)

Download the binary for your platform from the
[releases page](https://github.com/lennart-forgent/ghostty-web-server/releases) and run it:

```bash
chmod +x ghostty-web-server-linux-x64
./ghostty-web-server-linux-x64
```

Then open <http://localhost:8080>.

## Configuration

| Env | Default | Notes |
|---|---|---|
| `PORT` | `8080` | HTTP + WebSocket port |
| `SHELL` | `/bin/bash` | Shell to spawn for each session |

## Reverse proxy

HTTP and WebSocket share one port, and the client uses relative URLs, so it works behind
ngrok, nginx, or any proxy without extra config.

```nginx
location / {
    proxy_pass http://localhost:8080;
    proxy_http_version 1.1;
    proxy_set_header Upgrade $http_upgrade;
    proxy_set_header Connection "upgrade";
    proxy_set_header Host $host;
}
```

## Development

```bash
bun install
bun run dev       # http://localhost:8080, hot-reload for both server and client
```

`bun run dev` runs two watchers in parallel: `bun build --watch` for `src/client.tsx`
and `bun --watch` for `src/server.tsx`.

## Build

```bash
bun run build              # dist/server.js (Bun bundle, used as the npm bin)
bun run build:bin          # dist/ghostty-web-server (single binary, current platform)
bun run build:bin:linux-x64
bun run build:bin:linux-arm64
bun run build:bin:darwin-arm64
bun run build:bin:darwin-x64
```

## Architecture

```
src/
  components/App.tsx    server-rendered page shell (Preact JSX)
  islands/              client-hydrated components
    TerminalIsland.tsx  ghostty-web bootstrap + WebSocket PTY client
  client.tsx            hydration entry
  server.tsx            Elysia routes + Bun.spawn({ terminal }) PTY
```

Three flows from the same source:

| Mode | How | Output |
|---|---|---|
| Dev | `bun run dev` | live `bun --watch` of `src/server.tsx`, `bun build --watch` of `src/client.tsx` |
| npm | `bun run build` | `dist/server.js` + asset siblings (~1.5 MB total), Bun runtime required |
| Binary | `bun run build:bin:<plat>` | `dist/ghostty-web-server-<plat>` (~100 MB), self-contained |

PTY: `Bun.spawn({ terminal: { cols, rows, data } })` (Bun ≥ 1.3.5). No native dependencies.

## Security warning

⚠️ **This server provides full shell access.**
Only use for local development and demos. Do not expose to untrusted networks.
