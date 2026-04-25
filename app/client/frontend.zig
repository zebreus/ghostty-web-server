const zx = @import("zx");

pub fn start(session_id: anytype, terminal_text: anytype, state: anytype, message: anytype, take: bool) void {
    _ = session_id;
    _ = terminal_text;
    _ = take;
    state.set(.connected);
    message.set("Ziex client loaded; Ghostty WASM bridge pending browser WebSocket binding");
}

pub fn sendKey(event: zx.client.Event) void {
    _ = event;
}

pub fn paste(event: zx.client.Event) void {
    _ = event;
}
