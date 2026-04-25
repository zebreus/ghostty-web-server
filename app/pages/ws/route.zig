const zx = @import("zx");
const sessions = @import("../../server/sessions.zig");

pub fn GET(ctx: zx.RouteContext) !void {
    try ctx.socket.upgrade(sessions.requestFromQuery(ctx));
}

pub fn SocketOpen(ctx: zx.SocketOpenCtx(sessions.AttachRequest)) !void {
    try sessions.attach(ctx);
}

pub fn Socket(ctx: zx.SocketCtx(sessions.AttachRequest)) !void {
    try sessions.message(ctx);
}

pub fn SocketClose(ctx: zx.SocketCloseCtx(sessions.AttachRequest)) void {
    sessions.detach(ctx);
}
