const std = @import("std");

pub const TerminalFrontend = struct {
    cols: u16,
    rows: u16,

    pub fn init(_: std.mem.Allocator, cols: u16, rows: u16) !TerminalFrontend {
        return .{ .cols = cols, .rows = rows };
    }

    pub fn deinit(_: *TerminalFrontend) void {}

    pub fn resize(self: *TerminalFrontend, cols: u16, rows: u16) !void {
        self.cols = cols;
        self.rows = rows;
    }

    pub fn write(_: *TerminalFrontend, _: []const u8) void {}

    pub fn renderAlloc(terminal: *TerminalFrontend, allocator: std.mem.Allocator, _: anytype) ![]u8 {
        _ = terminal;
        return try allocator.dupe(u8, "");
    }

    pub fn title(_: *TerminalFrontend) ?[:0]const u8 {
        return null;
    }
};
