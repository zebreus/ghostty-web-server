const zx = @import("zx");
const sessions = @import("../../../server/sessions.zig");

pub fn GET(ctx: zx.RouteContext) !void {
    try sessions.writeSessionsJson(ctx);
}
