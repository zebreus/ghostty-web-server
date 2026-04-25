# ghostty-web-server

A local web server that opens a real shell session in your browser, rendered with the
[ghostty-web](https://github.com/coder/ghostty-web) terminal emulator. Built on
[actix-web](https://actix.rs) (server) and [Yew](https://yew.rs) (client, compiled to
WebAssembly). POSIX only (Linux + macOS).

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
| `RUST_LOG` | `info` | Standard `env_logger` filter |

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

You need a Rust toolchain (1.80+), the `wasm32-unknown-unknown` target, and
[trunk](https://trunkrs.dev):

```bash
rustup target add wasm32-unknown-unknown
cargo install --locked trunk
```

Then run two watchers in parallel:

```bash
# terminal 1 — Yew client (auto-rebuilds on save, output → client/dist/)
cd client && trunk watch

# terminal 2 — actix server (rebuild + restart on save)
cargo install cargo-watch          # one-time
cargo watch -x 'run -p ghostty-web-server'
```

Open <http://localhost:8080>.

The Nix flake provides a dev shell with all of the above:

```bash
nix develop
```

## Build

The binary embeds the Yew bundle, so the client must be built first:

```bash
( cd client && trunk build --release )
cargo build --release -p ghostty-web-server
# → target/release/ghostty-web-server
```

Cross-compilation for the four release targets uses
[`cargo zigbuild`](https://github.com/rust-cross/cargo-zigbuild) (also handled
in CI by `.github/workflows/publish.yml`):

```bash
cargo install cargo-zigbuild
rustup target add aarch64-unknown-linux-gnu x86_64-apple-darwin aarch64-apple-darwin

( cd client && trunk build --release )
for t in x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu \
         x86_64-apple-darwin     aarch64-apple-darwin; do
  cargo zigbuild --release --target "$t" -p ghostty-web-server
done
```

## Architecture

```
Cargo.toml                       workspace
server/                          actix-web binary
  src/main.rs                    routes, websocket, embedded asset serving
  src/session.rs                 PTY session manager (portable-pty)
  src/active_process.rs          /api/sessions helper
  src/protocol.rs                ServerMsg / ClientMsg JSON envelopes
  assets/                        embedded via rust-embed
    favicon.ico
    vendor/ghostty-web.js        the JS+WASM terminal renderer (vendored)
    vendor/ghostty-vt.wasm
client/                          Yew app (compiled to WASM)
  index.html                     trunk entry — static shell with three mount points
  src/main.rs                    mounts the three islands
  src/terminal_island.rs         ghostty-web bootstrap + WebSocket PTY client
  src/palette_island.rs          Cmd/Ctrl-K command palette
  src/status_island.rs           connection-status overlay
  src/bridge.rs                  mouse, paste, focus, title, bell wiring
  src/ghostty.rs                 wasm-bindgen interop with ghostty-web JS
```

PTY: [`portable-pty`](https://crates.io/crates/portable-pty) (the same library wezterm
uses). No external native runtime is required at runtime — everything is statically
compiled into one binary.

The `ghostty-web` JS+WASM library is **kept as-is** and called from the Yew client
through `wasm-bindgen` interop. There is no Rust port of it, and rewriting it would be a
much larger project than this server.

The wire protocol, environment variables, and HTTP routes are preserved bit-for-bit
from the original Bun + Elysia implementation, so any unchanged client artifact or
reverse-proxy configuration continues to work.

## Security warning

⚠️ **This server provides full shell access.**
Only use for local development and demos. Do not expose to untrusted networks.
