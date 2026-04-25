const zx = @import("zx");
const std = @import("std");

pub fn main() !void {
    const port = if (std.process.getEnvVarOwned(zx.allocator, "PORT")) |raw| blk: {
        defer zx.allocator.free(raw);
        break :blk std.fmt.parseInt(u16, raw, 10) catch 8080;
    } else |_| 8080;

    var app = try zx.App(void).init(zx.allocator, .{ .server = .{ .port = port } }, {});
    defer app.deinit();

    try app.start();
}

pub const std_options = zx.std_options;
