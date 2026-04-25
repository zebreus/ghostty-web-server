const std = @import("std");

// WASM-facing terminal surface. This module is intentionally isolated so it can
// be wired to ghostty/libghostty's terminal and renderer APIs without changing
// the Ziex app or WebSocket protocol.
var cols: u16 = 80;
var rows: u16 = 24;

export fn ghostty_terminal_init(initial_cols: u16, initial_rows: u16) void {
    cols = initial_cols;
    rows = initial_rows;
}

export fn ghostty_terminal_resize(next_cols: u16, next_rows: u16) void {
    cols = next_cols;
    rows = next_rows;
}

export fn ghostty_terminal_write(ptr: [*]const u8, len: usize) void {
    _ = ptr;
    _ = len;
}

export fn ghostty_terminal_cols() u16 {
    return cols;
}

export fn ghostty_terminal_rows() u16 {
    return rows;
}
