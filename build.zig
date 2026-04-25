const std = @import("std");
const zx = @import("zx");

pub fn build(b: *std.Build) !void {
    const target = b.standardTargetOptions(.{});
    const optimize = b.standardOptimizeOption(.{});

    const app_exe = b.addExecutable(.{
        .name = "ghostty-web-server",
        .root_module = b.createModule(.{
            .root_source_file = b.path("app/main.zig"),
            .target = target,
            .optimize = optimize,
        }),
    });

    const ghostty_wasm = b.addExecutable(.{
        .name = "ghostty-terminal",
        .root_module = b.createModule(.{
            .root_source_file = b.path("app/client/ghostty_terminal.zig"),
            .target = b.resolveTargetQuery(.{ .cpu_arch = .wasm32, .os_tag = .freestanding }),
            .optimize = optimize,
        }),
    });
    ghostty_wasm.rdynamic = true;

    const install_ghostty_wasm = b.addInstallArtifact(ghostty_wasm, .{ .dest_dir = .{ .override = .{ .custom = "public" } } });
    b.getInstallStep().dependOn(&install_ghostty_wasm.step);

    _ = try zx.init(b, app_exe, .{});
}
