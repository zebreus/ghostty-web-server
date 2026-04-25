const zx = @import("zx");
const sessions = @import("server/sessions.zig");

pub fn main() !void {
    try sessions.init(zx.allocator);
    defer sessions.deinit();

    var app = try zx.App(void).init(zx.allocator, .{}, {});
    defer app.deinit();

    try app.start();
}

pub const std_options = zx.std_options;
