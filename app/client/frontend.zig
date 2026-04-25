const zx = @import("zx");
const std = @import("std");
const builtin = @import("builtin");
const ghostty = if (builtin.target.cpu.arch.isWasm()) @import("ghostty_terminal.zig") else @import("ghostty_terminal_stub.zig");

pub const TerminalState = enum { connecting, connected, reconnecting, busy, disconnected };

const DEFAULT_COLS: u16 = 80;
const DEFAULT_ROWS: u16 = 24;
const MIN_COLS: u16 = 40;
const MIN_ROWS: u16 = 12;
const SCREEN_PADDING_X: i32 = 40;
const SCREEN_PADDING_Y: i32 = 32;

const TerminalSize = struct {
    cols: u16,
    rows: u16,
};

const Ui = struct {
    session_id: ?*zx.State([]const u8) = null,
    state: ?*zx.State(TerminalState) = null,
    message: ?*zx.State([]const u8) = null,
};

var ui: Ui = .{};
var socket: ?zx.WebSocket = null;
var terminal: ?ghostty.TerminalFrontend = null;
var resize_interval: ?u64 = null;
var terminal_size: TerminalSize = .{ .cols = DEFAULT_COLS, .rows = DEFAULT_ROWS };

pub fn start(session_id: *zx.State([]const u8), state: *zx.State(TerminalState), message: *zx.State([]const u8), take: bool) void {
    ui = .{
        .session_id = session_id,
        .state = state,
        .message = message,
    };

    state.set(.connecting);
    message.set("Connecting Ghostty terminal…");
    ensureResizePolling();
    terminal_size = measureTerminal();

    if (terminal) |*term| term.deinit();
    terminal = ghostty.TerminalFrontend.init(zx.allocator, terminal_size.cols, terminal_size.rows) catch {
        state.set(.disconnected);
        message.set("Failed to initialize Ghostty terminal");
        return;
    };
    renderTerminal();

    const sid = ensureSessionId() catch "default";
    session_id.set(sid);
    focusTerminal();
    connect(sid, take) catch {
        state.set(.disconnected);
        message.set("Failed to open WebSocket");
    };
}

pub fn sendKey(event: zx.client.Event) void {
    if (comptime !builtin.target.cpu.arch.isWasm()) return;

    event.preventDefault();
    const key = event.key() orelse return;
    const bytes = translateKey(key) orelse return;
    sendInput(bytes);
}

pub fn paste(event: zx.client.Event) void {
    if (comptime !builtin.target.cpu.arch.isWasm()) return;

    const js_event = event.getEvent();
    defer js_event.deinit();
    const clipboard = js_event.ref.get(zx.client.js.Object, "clipboardData") catch return;
    defer clipboard.deinit();
    const text = clipboard.callAlloc(zx.client.js.String, zx.allocator, "getData", .{zx.client.js.string("text")}) catch return;
    if (text.len == 0) return;

    event.preventDefault();
    sendInput(text);
}

fn connect(session_id: []const u8, take: bool) !void {
    if (socket) |*ws| ws.deinit();
    const url = try websocketUrl(session_id, take);
    socket = try zx.WebSocket.init(zx.allocator, url, .{});
    socket.?.onopen = onOpen;
    socket.?.onmessage = onMessage;
    socket.?.onerror = onError;
    socket.?.onclose = onClose;
    try socket.?.connect();
}

fn websocketUrl(session_id: []const u8, take: bool) ![]const u8 {
    const origin = try websocketOrigin();
    return try std.fmt.allocPrint(
        zx.allocator,
        "{s}/ws?sessionId={s}&cols={d}&rows={d}&take={d}",
        .{ origin, session_id, terminal_size.cols, terminal_size.rows, @intFromBool(take) },
    );
}

fn websocketOrigin() ![]const u8 {
    if (comptime !builtin.target.cpu.arch.isWasm()) return try zx.allocator.dupe(u8, "ws://localhost:8080");
    const location = zx.client.js.global.get(zx.client.js.Object, "location") catch return try zx.allocator.dupe(u8, "ws://localhost:8080");
    const origin = location.getAlloc(zx.client.js.String, zx.allocator, "origin") catch return try zx.allocator.dupe(u8, "ws://localhost:8080");
    if (std.mem.startsWith(u8, origin, "https://")) return try std.fmt.allocPrint(zx.allocator, "wss://{s}", .{origin["https://".len..]});
    if (std.mem.startsWith(u8, origin, "http://")) return try std.fmt.allocPrint(zx.allocator, "ws://{s}", .{origin["http://".len..]});
    return origin;
}

fn ensureSessionId() ![]const u8 {
    if (ui.session_id) |state| {
        if (state.value.len > 0) return state.value;
    }

    if (comptime !builtin.target.cpu.arch.isWasm()) return try zx.allocator.dupe(u8, "default");
    const key = "ghostty-web-session-id";
    const storage = zx.client.js.global.get(zx.client.js.Object, "localStorage") catch return try zx.allocator.dupe(u8, "default");
    const existing = storage.callAlloc(zx.client.js.String, zx.allocator, "getItem", .{zx.client.js.string(key)}) catch "";
    if (existing.len > 0) return existing;

    const value = if (zx.client.js.global.get(zx.client.js.Object, "crypto")) |crypto|
        crypto.callAlloc(zx.client.js.String, zx.allocator, "randomUUID", .{}) catch try zx.allocator.dupe(u8, "default")
    else |_|
        try zx.allocator.dupe(u8, "default");
    _ = storage.call(void, "setItem", .{ zx.client.js.string(key), zx.client.js.string(value) }) catch {};
    return value;
}

fn onOpen(_: *zx.WebSocket) void {
    if (ui.state) |state| state.set(.connected);
    if (ui.message) |message| message.set("Connected");
    syncTerminalSize(true);
    focusTerminal();
}

fn onError(_: *zx.WebSocket, event: zx.WebSocket.ErrorEvent) void {
    if (ui.state) |state| state.set(.disconnected);
    if (ui.message) |message| message.set(event.message);
}

fn onClose(_: *zx.WebSocket, _: zx.WebSocket.CloseEvent) void {
    if (ui.state) |state| state.set(.disconnected);
    if (ui.message) |message| message.set("Disconnected");
}

fn onMessage(_: *zx.WebSocket, event: zx.WebSocket.MessageEvent) void {
    const data = event.text() orelse return;
    const Envelope = struct {
        type: []const u8,
        value: ?[]const u8 = null,
        cols: ?u16 = null,
        rows: ?u16 = null,
    };
    const parsed = std.json.parseFromSlice(Envelope, zx.allocator, data, .{ .ignore_unknown_fields = true }) catch return;
    defer parsed.deinit();

    if (std.mem.eql(u8, parsed.value.type, "data")) {
        if (parsed.value.value) |chunk| {
            if (terminal) |*term| {
                term.write(chunk);
                if (term.title()) |title| {
                    if (ui.message) |message| message.set(title);
                    setDocumentTitle(title);
                }
                renderTerminal();
            }
        }
    } else if (std.mem.eql(u8, parsed.value.type, "ack")) {
        terminal_size = .{
            .cols = parsed.value.cols orelse terminal_size.cols,
            .rows = parsed.value.rows orelse terminal_size.rows,
        };
        if (ui.message) |message| {
            const status = std.fmt.allocPrint(zx.allocator, "Connected {d}×{d}", .{
                terminal_size.cols,
                terminal_size.rows,
            }) catch "Connected";
            message.set(status);
        }
    }
}

fn sendInput(bytes: []const u8) void {
    if (socket == null or !socket.?.isConnected()) return;
    var out: std.Io.Writer.Allocating = .init(zx.allocator);
    defer out.deinit();
    std.json.Stringify.value(.{ .type = "input", .value = bytes }, .{}, &out.writer) catch return;
    const payload = out.toOwnedSlice() catch return;
    socket.?.send(payload) catch {};
}

fn sendResize(cols: u16, rows: u16) void {
    if (socket == null or !socket.?.isConnected()) return;
    var out: std.Io.Writer.Allocating = .init(zx.allocator);
    defer out.deinit();
    std.json.Stringify.value(.{ .type = "resize", .cols = cols, .rows = rows }, .{}, &out.writer) catch return;
    const payload = out.toOwnedSlice() catch return;
    socket.?.send(payload) catch {};
}

fn renderTerminal() void {
    if (comptime !builtin.target.cpu.arch.isWasm()) return;
    const term = if (terminal) |*term| term else return;
    const rendered = term.renderAlloc(zx.allocator, .html) catch return;
    defer zx.allocator.free(rendered);

    var document = zx.client.Document.init(zx.allocator);
    defer document.deinit();
    const screen = document.getElementById("terminal-screen") catch return;
    defer screen.deinit();
    screen.setInnerHTML(rendered) catch return;

    const viewport = document.getElementById("terminal-viewport") catch return;
    defer viewport.deinit();
    const scroll_height = viewport.getProperty(i32, "scrollHeight") catch return;
    viewport.setProperty("scrollTop", scroll_height);
}

fn ensureResizePolling() void {
    if (comptime !builtin.target.cpu.arch.isWasm()) return;
    if (resize_interval) |id| zx.client.clearInterval(id);
    resize_interval = zx.client.setInterval(resizeTick, 250);
}

fn resizeTick() void {
    syncTerminalSize(false);
}

fn syncTerminalSize(force: bool) void {
    const size = measureTerminal();
    if (!force and size.cols == terminal_size.cols and size.rows == terminal_size.rows) return;
    terminal_size = size;

    if (terminal) |*term| {
        term.resize(size.cols, size.rows) catch {};
        renderTerminal();
    }
    sendResize(size.cols, size.rows);
}

fn measureTerminal() TerminalSize {
    if (comptime !builtin.target.cpu.arch.isWasm()) return .{ .cols = DEFAULT_COLS, .rows = DEFAULT_ROWS };

    var document = zx.client.Document.init(zx.allocator);
    defer document.deinit();
    const viewport = document.getElementById("terminal-viewport") catch return .{ .cols = DEFAULT_COLS, .rows = DEFAULT_ROWS };
    defer viewport.deinit();
    const ruler = document.getElementById("terminal-ruler") catch return .{ .cols = DEFAULT_COLS, .rows = DEFAULT_ROWS };
    defer ruler.deinit();

    const viewport_width = viewport.getProperty(i32, "clientWidth") catch return .{ .cols = DEFAULT_COLS, .rows = DEFAULT_ROWS };
    const viewport_height = viewport.getProperty(i32, "clientHeight") catch return .{ .cols = DEFAULT_COLS, .rows = DEFAULT_ROWS };
    const rect = ruler.ref.call(zx.client.js.Object, "getBoundingClientRect", .{}) catch return .{ .cols = DEFAULT_COLS, .rows = DEFAULT_ROWS };
    defer rect.deinit();
    const ruler_width = rect.get(f64, "width") catch return .{ .cols = DEFAULT_COLS, .rows = DEFAULT_ROWS };
    const ruler_height = rect.get(f64, "height") catch return .{ .cols = DEFAULT_COLS, .rows = DEFAULT_ROWS };
    if (ruler_width <= 0 or ruler_height <= 0) return .{ .cols = DEFAULT_COLS, .rows = DEFAULT_ROWS };

    const cell_width = ruler_width / 10.0;
    const cell_height = ruler_height;
    const usable_width = @max(1, viewport_width - SCREEN_PADDING_X);
    const usable_height = @max(1, viewport_height - SCREEN_PADDING_Y);

    const cols = @max(MIN_COLS, @as(u16, @intFromFloat(@floor(@as(f64, @floatFromInt(usable_width)) / cell_width))));
    const rows = @max(MIN_ROWS, @as(u16, @intFromFloat(@floor(@as(f64, @floatFromInt(usable_height)) / cell_height))));
    return .{ .cols = cols, .rows = rows };
}

fn focusTerminal() void {
    if (comptime !builtin.target.cpu.arch.isWasm()) return;
    var document = zx.client.Document.init(zx.allocator);
    defer document.deinit();
    const terminal_el = document.getElementById("terminal") catch return;
    defer terminal_el.deinit();
    _ = terminal_el.ref.call(zx.client.js.Object, "focus", .{}) catch {};
}

fn setDocumentTitle(title: []const u8) void {
    if (comptime !builtin.target.cpu.arch.isWasm()) return;
    var document = zx.client.Document.init(zx.allocator);
    defer document.deinit();
    document.ref.set("title", zx.client.js.string(title)) catch {};
}

fn translateKey(key: []const u8) ?[]const u8 {
    if (key.len == 1) {
        return key;
    }
    if (std.mem.eql(u8, key, "Enter")) return "\r";
    if (std.mem.eql(u8, key, "Backspace")) return "\x7f";
    if (std.mem.eql(u8, key, "Tab")) return "\t";
    if (std.mem.eql(u8, key, "Escape")) return "\x1b";
    if (std.mem.eql(u8, key, "ArrowUp")) return "\x1b[A";
    if (std.mem.eql(u8, key, "ArrowDown")) return "\x1b[B";
    if (std.mem.eql(u8, key, "ArrowRight")) return "\x1b[C";
    if (std.mem.eql(u8, key, "ArrowLeft")) return "\x1b[D";
    return null;
}
