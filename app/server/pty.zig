const std = @import("std");
const builtin = @import("builtin");

extern "c" fn openpty(amaster: *c_int, aslave: *c_int, name: ?[*]u8, termp: ?*anyopaque, winp: ?*std.posix.winsize) c_int;

pub const Pty = struct {
    allocator: std.mem.Allocator,
    master: std.fs.File,
    pid: std.posix.pid_t,

    pub fn spawn(allocator: std.mem.Allocator, cols: u16, rows: u16) !Pty {
        if (builtin.os.tag == .windows) return error.UnsupportedPlatform;

        const shell = std.process.getEnvVarOwned(allocator, "SHELL") catch try allocator.dupe(u8, "/bin/bash");
        defer allocator.free(shell);
        const shell_z = try allocator.dupeZ(u8, shell);
        defer allocator.free(shell_z);

        var master_fd: c_int = -1;
        var slave_fd: c_int = -1;
        var size: std.posix.winsize = .{
            .row = rows,
            .col = cols,
            .xpixel = 0,
            .ypixel = 0,
        };
        if (openpty(&master_fd, &slave_fd, null, null, &size) != 0) return error.OpenPtyFailed;
        errdefer std.posix.close(master_fd);
        errdefer std.posix.close(slave_fd);

        const pid = try std.posix.fork();
        if (pid == 0) {
            std.posix.close(master_fd);
            _ = std.posix.setsid() catch {};
            if (builtin.os.tag == .linux) {
                _ = std.posix.system.ioctl(slave_fd, std.os.linux.T.IOCSCTTY, @as(c_int, 0));
            }
            std.posix.dup2(slave_fd, std.posix.STDIN_FILENO) catch std.posix.exit(127);
            std.posix.dup2(slave_fd, std.posix.STDOUT_FILENO) catch std.posix.exit(127);
            std.posix.dup2(slave_fd, std.posix.STDERR_FILENO) catch std.posix.exit(127);
            if (slave_fd > std.posix.STDERR_FILENO) std.posix.close(slave_fd);

            const argv = [_:null]?[*:0]const u8{ shell_z.ptr, null };
            std.posix.execveZ(shell_z.ptr, &argv, std.c.environ) catch std.posix.exit(127);
        }

        std.posix.close(slave_fd);

        return .{
            .allocator = allocator,
            .master = .{ .handle = master_fd },
            .pid = pid,
        };
    }

    pub fn write(self: *Pty, bytes: []const u8) !void {
        try self.master.writeAll(bytes);
    }

    pub fn resize(self: *Pty, cols: u16, rows: u16) void {
        var size: std.posix.winsize = .{
            .row = rows,
            .col = cols,
            .xpixel = 0,
            .ypixel = 0,
        };
        _ = std.posix.system.ioctl(self.master.handle, std.posix.T.IOCSWINSZ, @intFromPtr(&size));
    }

    pub fn readLoop(self: *Pty, ctx: anytype, comptime onData: fn (@TypeOf(ctx), []const u8) void) void {
        var buf: [4096]u8 = undefined;
        while (true) {
            const n = self.master.read(&buf) catch break;
            if (n == 0) break;
            onData(ctx, buf[0..n]);
        }
    }

    pub fn deinit(self: *Pty) void {
        _ = std.posix.kill(self.pid, std.posix.SIG.TERM) catch {};
        _ = std.posix.waitpid(self.pid, 0);
        self.master.close();
    }
};
