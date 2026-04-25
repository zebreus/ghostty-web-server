const std = @import("std");
const builtin = @import("builtin");

const favicon = @embedFile("../../assets/favicon.ico");
const client_js = @embedFile("../client/bootstrap.js");

const SCROLLBACK_CAP = 256_000;
const SCROLLBACK_KEEP = 192_000;

const Winsize = extern struct {
    ws_row: c_ushort,
    ws_col: c_ushort,
    ws_xpixel: c_ushort,
    ws_ypixel: c_ushort,
};

extern "c" fn forkpty(amaster: *c_int, name: ?[*:0]u8, termp: ?*anyopaque, winp: ?*Winsize) c_int;
extern "c" fn setsid() c_int;

const Html =
    "<!doctype html>" ++
    "<html lang=\"en\"><head><meta charset=\"UTF-8\">" ++
    "<meta name=\"viewport\" content=\"width=device-width, initial-scale=1.0\">" ++
    "<link rel=\"icon\" type=\"image/vnd.microsoft.icon\" href=\"/favicon.ico\">" ++
    "<title>ghostty-web</title><style>" ++
    ":root{--bg:#1e1e1e;--fg:#e5e5e5;--accent:#4ea2ff;--warn:#e0b341;--mono:ui-monospace,Menlo,Monaco,monospace;--sans:-apple-system,BlinkMacSystemFont,Segoe UI,Roboto,sans-serif}" ++
    "html,body{margin:0;padding:0;height:100%;background:var(--bg);color:var(--fg);font-family:var(--sans);overflow:hidden}" ++
    "#terminal{width:100vw;height:100vh;height:100dvh}" ++
    ".terminal-screen{box-sizing:border-box;margin:0;width:100%;height:100%;padding:8px;overflow:auto;outline:none;white-space:pre-wrap;word-break:break-word;font:14px/17px var(--mono);background:#1e1e1e;color:#d4d4d4}" ++
    "#status{position:fixed;left:50%;top:18px;transform:translateX(-50%);z-index:10;color:#ddd;background:#262626;border:1px solid #353535;border-radius:8px;padding:8px 12px;font-size:13px}" ++
    "#status:empty{display:none}button{font:inherit;color:#fff;background:var(--accent);border:none;border-radius:6px;padding:6px 10px}" ++
    ".ghostty-bell-flash{animation:bell 180ms ease-out}@keyframes bell{40%{box-shadow:inset 0 0 0 9999px rgba(255,255,255,.07)}}" ++
    "</style></head><body><div id=\"terminal\"></div><div id=\"status\"></div><script type=\"module\" src=\"/client.js\"></script></body></html>";

const Session = struct {
    id: []const u8,
    started_at_ms: i64,
    pid: c_int,
    pty_fd: std.posix.fd_t,
    scrollback: std.ArrayList(u8),
    mutex: std.Thread.Mutex = .{},
    attached: ?*WebSocket = null,
    attach_key: u64 = 0,

    fn send(self: *Session, allocator: std.mem.Allocator, msg: []const u8) void {
        self.mutex.lock();
        defer self.mutex.unlock();
        if (self.attached) |ws| ws.sendText(allocator, msg) catch {};
    }

    fn appendScrollback(self: *Session, chunk: []const u8) void {
        self.mutex.lock();
        defer self.mutex.unlock();
        self.scrollback.appendSlice(chunk) catch return;
        if (self.scrollback.items.len > SCROLLBACK_CAP) {
            const keep = @min(self.scrollback.items.len, SCROLLBACK_KEEP);
            std.mem.copyForwards(u8, self.scrollback.items[0..keep], self.scrollback.items[self.scrollback.items.len - keep ..]);
            self.scrollback.shrinkRetainingCapacity(keep);
        }
    }
};

const App = struct {
    allocator: std.mem.Allocator,
    sessions: std.StringHashMap(*Session),
    mutex: std.Thread.Mutex = .{},

    fn init(allocator: std.mem.Allocator) App {
        return .{ .allocator = allocator, .sessions = std.StringHashMap(*Session).init(allocator) };
    }

    fn getOrCreateSession(self: *App, id: []const u8, cols: u16, rows: u16) !*Session {
        self.mutex.lock();
        defer self.mutex.unlock();
        if (self.sessions.get(id)) |existing| return existing;

        var ws: Winsize = .{ .ws_row = rows, .ws_col = cols, .ws_xpixel = 0, .ws_ypixel = 0 };
        var master: c_int = 0;
        const pid = forkpty(&master, null, null, &ws);
        if (pid < 0) return error.ForkPtyFailed;
        if (pid == 0) {
            _ = setsid();
            const shell = std.posix.getenv("SHELL") orelse "/bin/bash";
            const home = std.posix.getenv("HOME") orelse "/";
            std.posix.chdir(home) catch {};
            const argv = [_:null]?[*:0]const u8{ @ptrCast(shell.ptr), null };
            const envp = [_:null]?[*:0]const u8{ @ptrCast("TERM=xterm-256color"), @ptrCast("COLORTERM=truecolor"), null };
            std.posix.execveZ(@ptrCast(shell.ptr), &argv, &envp) catch std.posix.exit(127);
        }

        const owned_id = try self.allocator.dupe(u8, id);
        const session = try self.allocator.create(Session);
        session.* = .{
            .id = owned_id,
            .started_at_ms = std.time.milliTimestamp(),
            .pid = pid,
            .pty_fd = master,
            .scrollback = std.ArrayList(u8).init(self.allocator),
        };
        try self.sessions.put(owned_id, session);
        _ = try std.Thread.spawn(.{}, readPtyLoop, .{ self, session });
        return session;
    }

    fn removeSession(self: *App, session: *Session) void {
        self.mutex.lock();
        defer self.mutex.unlock();
        _ = self.sessions.remove(session.id);
    }
};

const WebSocket = struct {
    stream: std.net.Stream,
    mutex: std.Thread.Mutex = .{},
    closed: bool = false,

    fn sendText(self: *WebSocket, allocator: std.mem.Allocator, payload: []const u8) !void {
        self.mutex.lock();
        defer self.mutex.unlock();
        if (self.closed) return;
        var frame = std.ArrayList(u8).init(allocator);
        defer frame.deinit();
        try frame.append(0x81);
        if (payload.len < 126) {
            try frame.append(@intCast(payload.len));
        } else if (payload.len <= 0xffff) {
            try frame.append(126);
            try frame.writer().writeInt(u16, @intCast(payload.len), .big);
        } else {
            try frame.append(127);
            try frame.writer().writeInt(u64, @intCast(payload.len), .big);
        }
        try frame.appendSlice(payload);
        try self.stream.writeAll(frame.items);
    }

    fn close(self: *WebSocket, code: u16, reason: []const u8) void {
        self.mutex.lock();
        defer self.mutex.unlock();
        if (self.closed) return;
        self.closed = true;
        var buf: [128]u8 = undefined;
        var out = std.ArrayList(u8).init(std.heap.page_allocator);
        defer out.deinit();
        out.writer().writeInt(u16, code, .big) catch {};
        out.appendSlice(reason) catch {};
        const payload = out.items;
        buf[0] = 0x88;
        buf[1] = @intCast(payload.len);
        std.mem.copyForwards(u8, buf[2 .. 2 + payload.len], payload);
        self.stream.writeAll(buf[0 .. 2 + payload.len]) catch {};
        self.stream.close();
    }
};

pub fn main() !void {
    var gpa = std.heap.GeneralPurposeAllocator(.{}){};
    defer _ = gpa.deinit();
    const allocator = gpa.allocator();

    var app = App.init(allocator);
    const port = parsePort(std.posix.getenv("PORT") orelse "8080");
    const address = try std.net.Address.parseIp("0.0.0.0", port);
    var server = try address.listen(.{ .reuse_address = true });
    defer server.deinit();

    std.log.info("ghostty-web-server -> http://localhost:{d}", .{port});
    while (true) {
        const conn = try server.accept();
        _ = try std.Thread.spawn(.{}, handleConnection, .{ allocator, &app, conn.stream });
    }
}

fn handleConnection(allocator: std.mem.Allocator, app: *App, stream: std.net.Stream) void {
    var s = stream;
    defer s.close();
    var buffer: [16 * 1024]u8 = undefined;
    const n = s.read(&buffer) catch return;
    if (n == 0) return;
    const req = buffer[0..n];
    const head_end = std.mem.indexOf(u8, req, "\r\n\r\n") orelse return;
    const head = req[0..head_end];
    const first_line_end = std.mem.indexOf(u8, head, "\r\n") orelse return;
    const first = head[0..first_line_end];
    var parts = std.mem.splitScalar(u8, first, ' ');
    const method = parts.next() orelse return;
    const target = parts.next() orelse return;
    if (!std.mem.eql(u8, method, "GET")) {
        writeResponse(s, "405 Method Not Allowed", "text/plain", "method not allowed") catch return;
        return;
    }
    if (std.mem.startsWith(u8, target, "/ws?")) {
        websocket(allocator, app, s, head, target) catch return;
        return;
    }
    if (std.mem.eql(u8, target, "/") or std.mem.startsWith(u8, target, "/?")) return writeResponse(s, "200 OK", "text/html; charset=utf-8", Html) catch {};
    if (std.mem.eql(u8, target, "/client.js")) return writeResponse(s, "200 OK", "text/javascript; charset=utf-8", client_js) catch {};
    if (std.mem.eql(u8, target, "/favicon.ico")) return writeBytes(s, "200 OK", "image/vnd.microsoft.icon", favicon) catch {};
    if (std.mem.eql(u8, target, "/client.wasm")) return serveFile(s, "zig-out/bin/client.wasm", "application/wasm") catch writeResponse(s, "404 Not Found", "text/plain", "run `zig build` to create client.wasm") catch {};
    if (std.mem.eql(u8, target, "/api/sessions")) return sessionsJson(allocator, app, s) catch {};
    writeResponse(s, "404 Not Found", "text/plain", "not found") catch {};
}

fn websocket(allocator: std.mem.Allocator, app: *App, stream: std.net.Stream, head: []const u8, target: []const u8) !void {
    const key = headerValue(head, "Sec-WebSocket-Key") orelse return error.MissingWebSocketKey;
    try websocketAccept(stream, key);

    var query = Query.init(target[(std.mem.indexOfScalar(u8, target, '?') orelse 0) + 1 ..]);
    const id = query.get("sessionId") orelse "default";
    const cols = parseU16(query.get("cols") orelse "80", 80);
    const rows = parseU16(query.get("rows") orelse "24", 24);
    const take = std.mem.eql(u8, query.get("take") orelse "", "1");

    const session = try app.getOrCreateSession(id, cols, rows);
    var ws = WebSocket{ .stream = stream };
    var attach_key: u64 = 0;
    {
        session.mutex.lock();
        defer session.mutex.unlock();
        if (session.attached) |old| {
            if (take) old.close(1000, "taken") else {
                ws.close(4002, "session-busy");
                return;
            }
        }
        session.attached = &ws;
        session.attach_key += 1;
        attach_key = session.attach_key;
        resizePty(session.pty_fd, cols, rows) catch {};
        if (session.scrollback.items.len > 0) {
            const msg = try jsonData(allocator, session.scrollback.items);
            defer allocator.free(msg);
            try ws.sendText(allocator, msg);
        }
    }
    {
        const msg = try std.fmt.allocPrint(allocator, "{{\"type\":\"ack\",\"cols\":{d},\"rows\":{d}}}", .{ cols, rows });
        defer allocator.free(msg);
        try ws.sendText(allocator, msg);
    }
    while (true) {
        const payload = readFrame(allocator, stream) catch break;
        defer allocator.free(payload);
        if (parseInput(payload)) |input| {
            _ = std.posix.write(session.pty_fd, input) catch {};
        } else if (parseResize(payload)) |sz| {
            resizePty(session.pty_fd, sz.cols, sz.rows) catch {};
            const msg = try std.fmt.allocPrint(allocator, "{{\"type\":\"ack\",\"cols\":{d},\"rows\":{d}}}", .{ sz.cols, sz.rows });
            defer allocator.free(msg);
            try ws.sendText(allocator, msg);
        }
    }
    session.mutex.lock();
    if (session.attach_key == attach_key and session.attached == &ws) session.attached = null;
    session.mutex.unlock();
}

fn readPtyLoop(app: *App, session: *Session) void {
    var buf: [8192]u8 = undefined;
    while (true) {
        const n = std.posix.read(session.pty_fd, &buf) catch break;
        if (n == 0) break;
        session.appendScrollback(buf[0..n]);
        const msg = jsonData(app.allocator, buf[0..n]) catch continue;
        session.send(app.allocator, msg);
        app.allocator.free(msg);
    }
    session.send(app.allocator, "{\"type\":\"data\",\"value\":\"\\r\\n\\u001b[33mShell exited\\u001b[0m\\r\\n\"}");
    app.removeSession(session);
}

fn parseInput(payload: []const u8) ?[]const u8 {
    if (!std.mem.containsAtLeast(u8, payload, 1, "\"type\":\"input\"")) return null;
    return jsonStringValue(payload, "value");
}

const Size = struct { cols: u16, rows: u16 };
fn parseResize(payload: []const u8) ?Size {
    if (!std.mem.containsAtLeast(u8, payload, 1, "\"type\":\"resize\"")) return null;
    return .{ .cols = parseU16(jsonNumberValue(payload, "cols") orelse return null, 80), .rows = parseU16(jsonNumberValue(payload, "rows") orelse return null, 24) };
}

fn readFrame(allocator: std.mem.Allocator, stream: std.net.Stream) ![]u8 {
    var header: [2]u8 = undefined;
    try stream.reader().readNoEof(&header);
    const opcode = header[0] & 0x0f;
    if (opcode == 0x8) return error.Closed;
    const masked = (header[1] & 0x80) != 0;
    var len: u64 = header[1] & 0x7f;
    if (len == 126) len = try stream.reader().readInt(u16, .big);
    if (len == 127) len = try stream.reader().readInt(u64, .big);
    var mask: [4]u8 = .{ 0, 0, 0, 0 };
    if (masked) try stream.reader().readNoEof(&mask);
    const payload = try allocator.alloc(u8, @intCast(len));
    errdefer allocator.free(payload);
    try stream.reader().readNoEof(payload);
    if (masked) for (payload, 0..) |*b, i| b.* ^= mask[i % 4];
    return payload;
}

fn websocketAccept(stream: std.net.Stream, key: []const u8) !void {
    var sha = std.crypto.hash.Sha1.init(.{});
    sha.update(key);
    sha.update("258EAFA5-E914-47DA-95CA-C5AB0DC85B11");
    var digest: [20]u8 = undefined;
    sha.final(&digest);
    var out: [std.base64.standard.Encoder.calcSize(20)]u8 = undefined;
    const accept = std.base64.standard.Encoder.encode(&out, &digest);
    try stream.writeAll("HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: ");
    try stream.writeAll(accept);
    try stream.writeAll("\r\n\r\n");
}

fn headerValue(head: []const u8, name: []const u8) ?[]const u8 {
    var lines = std.mem.splitSequence(u8, head, "\r\n");
    _ = lines.next();
    while (lines.next()) |line| {
        if (line.len <= name.len + 1) continue;
        if (std.ascii.eqlIgnoreCase(line[0..name.len], name) and line[name.len] == ':') {
            return std.mem.trim(u8, line[name.len + 1 ..], " \t");
        }
    }
    return null;
}

const Query = struct {
    raw: []const u8,
    fn init(raw: []const u8) Query { return .{ .raw = raw }; }
    fn get(self: Query, name: []const u8) ?[]const u8 {
        var pairs = std.mem.splitScalar(u8, self.raw, '&');
        while (pairs.next()) |pair| {
            const eq = std.mem.indexOfScalar(u8, pair, '=') orelse continue;
            if (std.mem.eql(u8, pair[0..eq], name)) return pair[eq + 1 ..];
        }
        return null;
    }
};

fn jsonData(allocator: std.mem.Allocator, bytes: []const u8) ![]u8 {
    var out = std.ArrayList(u8).init(allocator);
    errdefer out.deinit();
    try out.appendSlice("{\"type\":\"data\",\"value\":\"");
    try jsonEscape(out.writer(), bytes);
    try out.appendSlice("\"}");
    return out.toOwnedSlice();
}

fn jsonEscape(writer: anytype, bytes: []const u8) !void {
    for (bytes) |b| switch (b) {
        '\\' => try writer.writeAll("\\\\"),
        '"' => try writer.writeAll("\\\""),
        '\n' => try writer.writeAll("\\n"),
        '\r' => try writer.writeAll("\\r"),
        '\t' => try writer.writeAll("\\t"),
        0x08 => try writer.writeAll("\\b"),
        0x0c => try writer.writeAll("\\f"),
        0...0x1f => try writer.print("\\u{x:0>4}", .{b}),
        else => try writer.writeByte(b),
    };
}

fn jsonStringValue(json: []const u8, name: []const u8) ?[]const u8 {
    const needle = std.fmt.allocPrint(std.heap.page_allocator, "\"{s}\":\"", .{name}) catch return null;
    defer std.heap.page_allocator.free(needle);
    const start = (std.mem.indexOf(u8, json, needle) orelse return null) + needle.len;
    var end = start;
    while (end < json.len) : (end += 1) {
        if (json[end] == '"' and json[end - 1] != '\\') return json[start..end];
    }
    return null;
}

fn jsonNumberValue(json: []const u8, name: []const u8) ?[]const u8 {
    const needle = std.fmt.allocPrint(std.heap.page_allocator, "\"{s}\":", .{name}) catch return null;
    defer std.heap.page_allocator.free(needle);
    var start = (std.mem.indexOf(u8, json, needle) orelse return null) + needle.len;
    while (start < json.len and json[start] == ' ') start += 1;
    var end = start;
    while (end < json.len and std.ascii.isDigit(json[end])) end += 1;
    return json[start..end];
}

fn resizePty(fd: std.posix.fd_t, cols: u16, rows: u16) !void {
    var ws = Winsize{ .ws_row = rows, .ws_col = cols, .ws_xpixel = 0, .ws_ypixel = 0 };
    _ = std.os.linux.ioctl(fd, 0x5414, @intFromPtr(&ws));
}

fn sessionsJson(allocator: std.mem.Allocator, app: *App, stream: std.net.Stream) !void {
    var out = std.ArrayList(u8).init(allocator);
    defer out.deinit();
    try out.appendSlice("{\"sessions\":[");
    app.mutex.lock();
    defer app.mutex.unlock();
    var it = app.sessions.valueIterator();
    var first = true;
    while (it.next()) |ptr| {
        const session = ptr.*;
        session.mutex.lock();
        defer session.mutex.unlock();
        if (!first) try out.append(',');
        first = false;
        try out.writer().print("{{\"id\":\"{s}\",\"startedAt\":{d},\"attached\":{},\"activeProcess\":\"{s}\"}}", .{ session.id, session.started_at_ms, session.attached != null, "(unknown)" });
    }
    try out.appendSlice("]}");
    try writeResponse(stream, "200 OK", "application/json", out.items);
}

fn serveFile(stream: std.net.Stream, path: []const u8, content_type: []const u8) !void {
    const file = try std.fs.cwd().openFile(path, .{});
    defer file.close();
    const data = try file.readToEndAlloc(std.heap.page_allocator, 16 * 1024 * 1024);
    defer std.heap.page_allocator.free(data);
    try writeBytes(stream, "200 OK", content_type, data);
}

fn writeResponse(stream: std.net.Stream, status: []const u8, content_type: []const u8, body: []const u8) !void {
    try writeBytes(stream, status, content_type, body);
}

fn writeBytes(stream: std.net.Stream, status: []const u8, content_type: []const u8, body: []const u8) !void {
    try stream.writer().print("HTTP/1.1 {s}\r\nContent-Type: {s}\r\nContent-Length: {d}\r\nConnection: close\r\n\r\n", .{ status, content_type, body.len });
    try stream.writeAll(body);
}

fn parsePort(s: []const u8) u16 {
    return std.fmt.parseInt(u16, s, 10) catch 8080;
}

fn parseU16(s: []const u8, fallback: u16) u16 {
    return std.fmt.parseInt(u16, s, 10) catch fallback;
}
