const std = @import("std");
const builtin = @import("builtin");
const vt = @import("ghostty-vt");

const Allocator = std.mem.Allocator;
const DEFAULT_SCROLLBACK_ROWS = 10_000;
const export_allocator: Allocator = if (builtin.target.cpu.arch.isWasm()) std.heap.wasm_allocator else std.heap.page_allocator;

pub const std_options = if (builtin.target.cpu.arch.isWasm()) vt.std_options else std.Options{};

pub const Format = enum(u8) {
    plain = 0,
    html = 1,
    vt = 2,
};

pub const TerminalFrontend = struct {
    allocator: Allocator,
    terminal: *vt.Terminal,
    stream: vt.TerminalStream,
    rows: u16,
    cols: u16,
    bell_count: u32 = 0,
    title_changed: bool = false,

    pub fn init(allocator: Allocator, cols: u16, rows: u16) !TerminalFrontend {
        if (cols == 0 or rows == 0) return error.InvalidTerminalSize;

        const terminal = try allocator.create(vt.Terminal);
        errdefer allocator.destroy(terminal);
        terminal.* = try vt.Terminal.init(allocator, .{
            .cols = cols,
            .rows = rows,
            .max_scrollback = DEFAULT_SCROLLBACK_ROWS,
        });
        errdefer terminal.deinit(allocator);

        var handler = terminal.vtHandler();
        handler.effects = .{
            .write_pty = null,
            .bell = bell,
            .color_scheme = null,
            .device_attributes = null,
            .enquiry = null,
            .size = null,
            .title_changed = titleChanged,
            .xtversion = null,
        };

        return .{
            .allocator = allocator,
            .terminal = terminal,
            .stream = vt.TerminalStream.initAlloc(allocator, handler),
            .cols = cols,
            .rows = rows,
        };
    }

    pub fn deinit(self: *TerminalFrontend) void {
        self.stream.deinit();
        self.terminal.deinit(self.allocator);
        self.allocator.destroy(self.terminal);
        self.* = undefined;
    }

    pub fn resize(self: *TerminalFrontend, cols: u16, rows: u16) !void {
        if (cols == 0 or rows == 0) return error.InvalidTerminalSize;
        try self.terminal.resize(self.allocator, cols, rows);
        self.cols = cols;
        self.rows = rows;
    }

    pub fn write(self: *TerminalFrontend, bytes: []const u8) void {
        self.stream.nextSlice(bytes);
    }

    pub fn renderAlloc(self: *TerminalFrontend, allocator: Allocator, format: Format) ![]u8 {
        var formatter: vt.formatter.TerminalFormatter = .init(self.terminal, .{
            .emit = switch (format) {
                .plain => .plain,
                .html => .html,
                .vt => .vt,
            },
            .unwrap = false,
            .trim = true,
        });
        formatter.extra = switch (format) {
            .plain => .none,
            .html, .vt => .styles,
        };

        var out: std.Io.Writer.Allocating = .init(allocator);
        defer out.deinit();
        try formatter.format(&out.writer);
        return out.toOwnedSlice();
    }

    pub fn title(self: *TerminalFrontend) ?[:0]const u8 {
        return self.terminal.getTitle();
    }

    fn fromHandler(handler: *vt.TerminalStream.Handler) *TerminalFrontend {
        const stream: *vt.TerminalStream = @fieldParentPtr("handler", handler);
        return @fieldParentPtr("stream", stream);
    }

    fn bell(handler: *vt.TerminalStream.Handler) void {
        const self = fromHandler(handler);
        self.bell_count += 1;
    }

    fn titleChanged(handler: *vt.TerminalStream.Handler) void {
        const self = fromHandler(handler);
        self.title_changed = true;
    }
};

var global: ?TerminalFrontend = null;

fn ensureGlobal() !*TerminalFrontend {
    if (global == null) {
        global = try TerminalFrontend.init(export_allocator, 80, 24);
    }
    return &global.?;
}

export fn ghostty_terminal_init(initial_cols: u16, initial_rows: u16) void {
    if (global) |*existing| existing.deinit();
    global = TerminalFrontend.init(export_allocator, initial_cols, initial_rows) catch null;
}

export fn ghostty_terminal_resize(next_cols: u16, next_rows: u16) void {
    const terminal = ensureGlobal() catch return;
    terminal.resize(next_cols, next_rows) catch {};
}

export fn ghostty_terminal_write(ptr: [*]const u8, len: usize) void {
    const terminal = ensureGlobal() catch return;
    terminal.write(ptr[0..len]);
}

export fn ghostty_terminal_cols() u16 {
    const terminal = ensureGlobal() catch return 0;
    return terminal.cols;
}

export fn ghostty_terminal_rows() u16 {
    const terminal = ensureGlobal() catch return 0;
    return terminal.rows;
}

export fn ghostty_terminal_bell_count() u32 {
    const terminal = ensureGlobal() catch return 0;
    return terminal.bell_count;
}

export fn ghostty_terminal_title_ptr() [*]const u8 {
    const terminal = ensureGlobal() catch return "";
    return (terminal.title() orelse "").ptr;
}

export fn ghostty_terminal_title_len() usize {
    const terminal = ensureGlobal() catch return 0;
    return (terminal.title() orelse "").len;
}

export fn ghostty_terminal_render_len(format: Format) usize {
    const terminal = ensureGlobal() catch return 0;
    const rendered = terminal.renderAlloc(export_allocator, format) catch return 0;
    defer export_allocator.free(rendered);
    return rendered.len;
}

export fn ghostty_terminal_render(format: Format, out_ptr: [*]u8, out_len: usize) usize {
    const terminal = ensureGlobal() catch return 0;
    const rendered = terminal.renderAlloc(export_allocator, format) catch return 0;
    defer export_allocator.free(rendered);
    const n = @min(out_len, rendered.len);
    @memcpy(out_ptr[0..n], rendered[0..n]);
    return n;
}

test "Ghostty terminal parses text, title, bell, resize, and HTML render" {
    var terminal = try TerminalFrontend.init(std.testing.allocator, 8, 3);
    defer terminal.deinit();

    terminal.write("Hello");
    terminal.write("\x07");
    terminal.write("\x1b]0;Ghostty Web\x07");

    const plain = try terminal.renderAlloc(std.testing.allocator, .plain);
    defer std.testing.allocator.free(plain);
    try std.testing.expect(std.mem.indexOf(u8, plain, "Hello") != null);
    try std.testing.expectEqual(@as(u32, 1), terminal.bell_count);
    try std.testing.expectEqualStrings("Ghostty Web", terminal.title() orelse "");

    try terminal.resize(12, 4);
    try std.testing.expectEqual(@as(u16, 12), terminal.cols);
    try std.testing.expectEqual(@as(u16, 4), terminal.rows);

    terminal.write("\x1b[31mred\x1b[0m");
    const html = try terminal.renderAlloc(std.testing.allocator, .html);
    defer std.testing.allocator.free(html);
    try std.testing.expect(std.mem.indexOf(u8, html, "red") != null);
    try std.testing.expect(std.mem.indexOf(u8, html, "<") != null);
}
