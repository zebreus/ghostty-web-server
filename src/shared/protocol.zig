pub const ServerMessage = union(enum) {
    data: []const u8,
    ack: Size,
};

pub const ClientMessage = union(enum) {
    input: []const u8,
    resize: Size,
};

pub const Size = struct {
    cols: u16,
    rows: u16,
};
