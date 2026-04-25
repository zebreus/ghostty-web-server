const zx = @import("zx");
const std = @import("std");
const builtin = @import("builtin");
const ghostty = if (builtin.target.cpu.arch.isWasm()) @import("ghostty_terminal.zig") else @import("ghostty_terminal_stub.zig");

pub const TerminalState = enum { connecting, connected, reconnecting, busy, disconnected };

const Ui = struct {
    session_id: ?*zx.State([]const u8) = null,
    terminal_text: ?*zx.State([]const u8) = null,
    state: ?*zx.State(TerminalState) = null,
    message: ?*zx.State([]const u8) = null,
};

var ui: Ui = .{};
var socket: ?zx.WebSocket = null;
var terminal: ?ghostty.TerminalFrontend = null;

pub fn start(session_id: *zx.State([]const u8), terminal_text: *zx.State([]const u8), state: *zx.State(TerminalState), message: *zx.State([]const u8), take: bool) void {
    ui = .{
        .session_id = session_id,
        .terminal_text = terminal_text,
        .state = state,
        .message = message,
    };

    state.set(.connecting);
    message.set("Connecting Ghostty terminal…");

    if (terminal) |*term| term.deinit();
    terminal = ghostty.TerminalFrontend.init(zx.allocator, 80, 24) catch {
        state.set(.disconnected);
        message.set("Failed to initialize Ghostty terminal");
        return;
    };

    const sid = ensureSessionId() catch "default";
    session_id.set(sid);
    connect(sid, take) catch {
        state.set(.disconnected);
        message.set("Failed to open WebSocket");
    };
}

pub fn sendKey(event: zx.client.Event) void {
    if (comptime !builtin.target.cpu.arch.isWasm()) {
        return;
    }

    event.preventDefault();
    const key = event.key() orelse return;
    const bytes = translateKey(key) orelse return;
    sendInput(bytes);
}

pub fn paste(event: zx.client.Event) void {
    _ = event;
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
        "{s}/ws?sessionId={s}&cols=80&rows=24&take={d}",
        .{ origin, session_id, @intFromBool(take) },
    );
}

fn websocketOrigin() ![]const u8 {
    if (comptime !builtin.target.cpu.arch.isWasm()) return try zx.allocator.dupe(u8, "ws://localhost:8080");
    const js = zx.client.js;
    const location = js.global.get(js.Object, "location") catch return try zx.allocator.dupe(u8, "ws://localhost:8080");
    const origin = location.getAlloc(js.String, zx.allocator, "origin") catch return try zx.allocator.dupe(u8, "ws://localhost:8080");
    if (std.mem.startsWith(u8, origin, "https://")) return try std.fmt.allocPrint(zx.allocator, "wss://{s}", .{origin["https://".len..]});
    if (std.mem.startsWith(u8, origin, "http://")) return try std.fmt.allocPrint(zx.allocator, "ws://{s}", .{origin["http://".len..]});
    return origin;
}

fn ensureSessionId() ![]const u8 {
    if (ui.session_id) |state| {
        if (state.value.len > 0) return state.value;
    }

    if (comptime !builtin.target.cpu.arch.isWasm()) return try zx.allocator.dupe(u8, "default");
    const js = zx.client.js;
    const key = "ghostty-web-session-id";
    const storage = js.global.get(js.Object, "localStorage") catch return try zx.allocator.dupe(u8, "default");
    const existing = storage.callAlloc(js.String, zx.allocator, "getItem", .{js.String.init(key)}) catch "";
    if (existing.len > 0) return existing;

    const value = if (js.global.get(js.Object, "crypto")) |crypto|
        crypto.callAlloc(js.String, zx.allocator, "randomUUID", .{}) catch try zx.allocator.dupe(u8, "default")
    else |_|
        try zx.allocator.dupe(u8, "default");
    _ = storage.call(void, "setItem", .{ js.String.init(key), js.String.init(value) }) catch {};
    return value;
}

fn onOpen(_: *zx.WebSocket) void {
    if (ui.state) |state| state.set(.connected);
    if (ui.message) |message| message.set("Connected");
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
                const rendered = term.renderAlloc(zx.allocator, .plain) catch return;
                if (ui.terminal_text) |text| text.set(rendered);
                if (term.title()) |title| {
                    if (ui.message) |message| message.set(title);
                }
            }
        }
    } else if (std.mem.eql(u8, parsed.value.type, "ack")) {
        if (ui.message) |message| {
            const status = std.fmt.allocPrint(zx.allocator, "Connected {d}×{d}", .{
                parsed.value.cols orelse 80,
                parsed.value.rows orelse 24,
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

fn translateKey(key: []const u8) ?[]const u8 {
    if (key.len == 1) return key;
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
