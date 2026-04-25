const std = @import("std");

extern "env" fn js_render_text(ptr: [*]const u8, len: usize) void;
extern "env" fn js_set_title(ptr: [*]const u8, len: usize) void;
extern "env" fn js_bell() void;

var cols: u32 = 80;
var rows: u32 = 24;

export fn terminal_init(initial_cols: u32, initial_rows: u32) void {
    cols = initial_cols;
    rows = initial_rows;
}

export fn terminal_resize(next_cols: u32, next_rows: u32) void {
    cols = next_cols;
    rows = next_rows;
}

export fn terminal_feed(ptr: [*]const u8, len: usize) void {
    const data = ptr[0..len];
    var i: usize = 0;
    while (i < data.len) : (i += 1) {
        if (data[i] == 0x07) js_bell();
    }
    js_render_text(data.ptr, data.len);
}

export fn terminal_cols() u32 {
    return cols;
}

export fn terminal_rows() u32 {
    return rows;
}

export fn terminal_set_title(ptr: [*]const u8, len: usize) void {
    js_set_title(ptr, len);
}

export fn alloc(len: usize) ?[*]u8 {
    const bytes = std.heap.wasm_allocator.alloc(u8, len) catch return null;
    return bytes.ptr;
}

export fn free(ptr: [*]u8, len: usize) void {
    std.heap.wasm_allocator.free(ptr[0..len]);
}
