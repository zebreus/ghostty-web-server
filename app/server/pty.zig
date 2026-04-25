const std = @import("std");
const builtin = @import("builtin");

pub const Pty = struct {
    allocator: std.mem.Allocator,
    child: std.process.Child,
    stdin: std.fs.File,
    stdout: std.fs.File,

    pub fn spawn(allocator: std.mem.Allocator, cols: u16, rows: u16) !Pty {
        _ = cols;
        _ = rows;
        const shell = std.process.getEnvVarOwned(allocator, "SHELL") catch try allocator.dupe(u8, "/bin/bash");
        defer allocator.free(shell);

        var env = try std.process.getEnvMap(allocator);
        errdefer env.deinit();
        try env.put("TERM", "xterm-256color");
        try env.put("COLORTERM", "truecolor");

        var child = std.process.Child.init(&.{shell}, allocator);
        child.stdin_behavior = .Pipe;
        child.stdout_behavior = .Pipe;
        child.stderr_behavior = .Pipe;
        child.env_map = &env;
        child.cwd = std.process.getEnvVarOwned(allocator, "HOME") catch null;
        try child.spawn();

        return .{
            .allocator = allocator,
            .child = child,
            .stdin = child.stdin.?,
            .stdout = child.stdout.?,
        };
    }

    pub fn write(self: *Pty, bytes: []const u8) !void {
        try self.stdin.writeAll(bytes);
    }

    pub fn resize(self: *Pty, cols: u16, rows: u16) void {
        _ = self;
        _ = cols;
        _ = rows;
    }

    pub fn readLoop(self: *Pty, ctx: anytype, comptime onData: fn (@TypeOf(ctx), []const u8) void) void {
        var buf: [4096]u8 = undefined;
        while (true) {
            const n = self.stdout.read(&buf) catch break;
            if (n == 0) break;
            onData(ctx, buf[0..n]);
        }
    }

    pub fn deinit(self: *Pty) void {
        _ = self.child.kill() catch {};
        _ = self.child.wait() catch {};
        self.stdin.close();
        self.stdout.close();
    }
};
