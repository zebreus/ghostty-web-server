const std = @import("std");
const zx = @import("zx");
const protocol = @import("protocol.zig");
const Pty = @import("pty.zig").Pty;

const SCROLLBACK_CAP = 256_000;
const SCROLLBACK_KEEP = 192_000;

pub const AttachRequest = extern struct {
    session_id: [64]u8,
    session_len: u8,
    cols: u16,
    rows: u16,
    take: bool,
};

const Attached = struct {
    socket: zx.Socket,
    key: u64,
};

const Session = struct {
    id: []const u8,
    started_at: i64,
    pty: Pty,
    scrollback: std.ArrayListUnmanaged(u8) = .{},
    attached: ?Attached = null,
    attach_key: u64 = 0,
    reader: ?std.Thread = null,
};

var allocator_: ?std.mem.Allocator = null;
var mutex: std.Thread.Mutex = .{};
var sessions: std.StringHashMapUnmanaged(*Session) = .{};

pub fn deinit() void {
    const allocator = allocator_ orelse return;
    mutex.lock();
    defer mutex.unlock();
    var it = sessions.iterator();
    while (it.next()) |entry| {
        var s = entry.value_ptr.*;
        s.pty.deinit();
        s.scrollback.deinit(allocator);
        allocator.free(s.id);
        allocator.destroy(s);
    }
    sessions.deinit(allocator);
}

pub fn requestFromQuery(ctx: zx.RouteContext) AttachRequest {
    var req = AttachRequest{ .session_id = [_]u8{0} ** 64, .session_len = 0, .cols = 80, .rows = 24, .take = false };
    if (ctx.request.queries.get("sessionId")) |id| {
        const n = @min(id.len, req.session_id.len);
        @memcpy(req.session_id[0..n], id[0..n]);
        req.session_len = @intCast(n);
    }
    if (ctx.request.queries.get("cols")) |cols| req.cols = std.fmt.parseInt(u16, cols, 10) catch 80;
    if (ctx.request.queries.get("rows")) |rows| req.rows = std.fmt.parseInt(u16, rows, 10) catch 24;
    req.take = if (ctx.request.queries.get("take")) |take| std.mem.eql(u8, take, "1") else false;
    return req;
}

pub fn attach(ctx: zx.SocketOpenCtx(AttachRequest)) !void {
    const allocator = ensureAllocator(ctx.allocator);
    const id = ctx.data.session_id[0..ctx.data.session_len];
    if (id.len == 0) {
        ctx.socket.close();
        return;
    }

    mutex.lock();
    var s = sessions.get(id) orelse createSessionLocked(id, ctx.data.cols, ctx.data.rows) catch |err| {
        mutex.unlock();
        return err;
    };

    if (s.attached) |attached| {
        if (ctx.data.take) attached.socket.close() else {
            mutex.unlock();
            ctx.socket.close();
            return;
        }
    }

    s.attach_key += 1;
    s.attached = .{ .socket = ctx.socket, .key = s.attach_key };
    s.pty.resize(ctx.data.cols, ctx.data.rows);
    const scrollback = try allocator.dupe(u8, s.scrollback.items);
    mutex.unlock();
    defer allocator.free(scrollback);

    if (scrollback.len > 0) try send(ctx.socket, .{ .data = .{ .value = scrollback } });
    try send(ctx.socket, .{ .ack = .{ .cols = ctx.data.cols, .rows = ctx.data.rows } });
}

pub fn detach(ctx: zx.SocketCloseCtx(AttachRequest)) void {
    const id = ctx.data.session_id[0..ctx.data.session_len];
    mutex.lock();
    defer mutex.unlock();
    if (sessions.get(id)) |s| {
        if (s.attached != null) s.attached = null;
    }
}

pub fn message(ctx: zx.SocketCtx(AttachRequest)) !void {
    const id = ctx.data.session_id[0..ctx.data.session_len];
    const msg = protocol.parseClient(ctx.arena, ctx.message) catch return;

    mutex.lock();
    const s = sessions.get(id) orelse {
        mutex.unlock();
        return;
    };
    switch (msg) {
        .input => |input| try s.pty.write(input.value),
        .resize => |resize| {
            s.pty.resize(resize.cols, resize.rows);
            mutex.unlock();
            try send(ctx.socket, .{ .ack = .{ .cols = resize.cols, .rows = resize.rows } });
            return;
        },
    }
    mutex.unlock();
}

pub fn writeSessionsJson(ctx: zx.RouteContext) !void {
    mutex.lock();
    defer mutex.unlock();
    var list: std.ArrayList(struct { id: []const u8, startedAt: i64, attached: bool, activeProcess: []const u8 }) = .empty;
    var it = sessions.iterator();
    while (it.next()) |entry| {
        try list.append(ctx.arena, .{
            .id = entry.value_ptr.*.id,
            .startedAt = entry.value_ptr.*.started_at,
            .attached = entry.value_ptr.*.attached != null,
            .activeProcess = "shell",
        });
    }
    try ctx.response.json(.{ .sessions = list.items }, .{});
}

fn createSessionLocked(id: []const u8, cols: u16, rows: u16) !*Session {
    const allocator = allocator_.?;
    const owned_id = try allocator.dupe(u8, id);
    errdefer allocator.free(owned_id);
    const s = try allocator.create(Session);
    errdefer allocator.destroy(s);
    s.* = .{
        .id = owned_id,
        .started_at = std.time.milliTimestamp(),
        .pty = try Pty.spawn(allocator, cols, rows),
    };
    try sessions.put(allocator, owned_id, s);
    s.reader = try std.Thread.spawn(.{}, readerMain, .{s});
    return s;
}

fn readerMain(s: *Session) void {
    s.pty.readLoop(s, onPtyData);
    mutex.lock();
    const attached = s.attached;
    _ = sessions.remove(s.id);
    mutex.unlock();
    if (attached) |a| {
        send(a.socket, .{ .data = .{ .value = "\r\n\x1b[33mShell exited\x1b[0m\r\n" } }) catch {};
        a.socket.close();
    }
}

fn onPtyData(s: *Session, data: []const u8) void {
    mutex.lock();
    const allocator = allocator_.?;
    s.scrollback.appendSlice(allocator, data) catch {};
    if (s.scrollback.items.len > SCROLLBACK_CAP) {
        const keep_from = s.scrollback.items.len - SCROLLBACK_KEEP;
        std.mem.copyForwards(u8, s.scrollback.items[0..SCROLLBACK_KEEP], s.scrollback.items[keep_from..]);
        s.scrollback.shrinkRetainingCapacity(SCROLLBACK_KEEP);
    }
    const attached = s.attached;
    mutex.unlock();
    if (attached) |a| send(a.socket, .{ .data = .{ .value = data } }) catch {};
}

fn send(socket: zx.Socket, msg: protocol.ServerMsg) !void {
    const allocator = allocator_.?;
    const json = try protocol.stringifyServer(allocator, msg);
    defer allocator.free(json);
    try socket.write(json);
}

fn ensureAllocator(allocator: std.mem.Allocator) std.mem.Allocator {
    if (allocator_ == null) allocator_ = allocator;
    return allocator_.?;
}
