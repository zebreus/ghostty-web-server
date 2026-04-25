const std = @import("std");

pub const ServerMsg = union(enum) {
    data: struct { value: []const u8 },
    ack: struct { cols: u16, rows: u16 },
};

pub const ClientMsg = union(enum) {
    input: struct { value: []const u8 },
    resize: struct { cols: u16, rows: u16 },
};

pub fn stringifyServer(allocator: std.mem.Allocator, msg: ServerMsg) ![]u8 {
    var out: std.Io.Writer.Allocating = .init(allocator);
    defer out.deinit();
    switch (msg) {
        .data => |d| try std.json.Stringify.value(.{ .type = "data", .value = d.value }, .{}, &out.writer),
        .ack => |a| try std.json.Stringify.value(.{ .type = "ack", .cols = a.cols, .rows = a.rows }, .{}, &out.writer),
    }
    return out.toOwnedSlice();
}

pub fn parseClient(allocator: std.mem.Allocator, raw: []const u8) !ClientMsg {
    const Envelope = struct {
        type: []const u8,
        value: ?[]const u8 = null,
        cols: ?u16 = null,
        rows: ?u16 = null,
    };
    const parsed = try std.json.parseFromSlice(Envelope, allocator, raw, .{ .ignore_unknown_fields = true });
    defer parsed.deinit();
    const env = parsed.value;
    if (std.mem.eql(u8, env.type, "input")) {
        return .{ .input = .{ .value = env.value orelse "" } };
    }
    if (std.mem.eql(u8, env.type, "resize")) {
        return .{ .resize = .{ .cols = env.cols orelse 80, .rows = env.rows orelse 24 } };
    }
    return error.UnknownClientMessage;
}
