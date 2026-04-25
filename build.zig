const std = @import("std");
const zx = @import("zx");

pub fn build(b: *std.Build) !void {
    const target = b.standardTargetOptions(.{});
    const optimize = b.standardOptimizeOption(.{});
    const wasm_target = b.resolveTargetQuery(.{ .cpu_arch = .wasm32, .os_tag = .freestanding });

    const ghostty_native = b.dependency("ghostty", .{
        .target = target,
        .optimize = optimize,
        .@"emit-lib-vt" = true,
        .@"emit-terminfo" = false,
        .@"emit-termcap" = false,
        .@"emit-themes" = false,
        .i18n = false,
        .simd = false,
    });
    const ghostty_wasm_dep = b.dependency("ghostty", .{
        .target = wasm_target,
        .optimize = optimize,
        .@"emit-lib-vt" = true,
        .@"emit-terminfo" = false,
        .@"emit-termcap" = false,
        .@"emit-themes" = false,
        .i18n = false,
        .simd = false,
    });

    const app_exe = b.addExecutable(.{
        .name = "ghostty-web-server",
        .root_module = b.createModule(.{
            .root_source_file = b.path("app/main.zig"),
            .target = target,
            .optimize = optimize,
        }),
    });
    app_exe.root_module.addImport("ghostty-vt", ghostty_wasm_dep.module("ghostty-vt"));

    const ghostty_wasm = b.addExecutable(.{
        .name = "ghostty-terminal",
        .root_module = b.createModule(.{
            .root_source_file = b.path("app/client/ghostty_terminal.zig"),
            .target = wasm_target,
            .optimize = optimize,
            .single_threaded = true,
        }),
    });
    ghostty_wasm.root_module.addImport("ghostty-vt", ghostty_wasm_dep.module("ghostty-vt"));
    ghostty_wasm.entry = .disabled;
    ghostty_wasm.rdynamic = true;

    const install_ghostty_wasm = b.addInstallArtifact(ghostty_wasm, .{ .dest_dir = .{ .override = .{ .custom = "public" } } });
    b.getInstallStep().dependOn(&install_ghostty_wasm.step);

    const ghostty_tests = b.addTest(.{
        .name = "ghostty-terminal-test",
        .root_module = b.createModule(.{
            .root_source_file = b.path("app/client/ghostty_terminal.zig"),
            .target = target,
            .optimize = optimize,
        }),
    });
    ghostty_tests.root_module.addImport("ghostty-vt", ghostty_native.module("ghostty-vt"));
    const run_ghostty_tests = b.addRunArtifact(ghostty_tests);
    const test_step = b.step("test", "Run Ghostty terminal integration tests");
    test_step.dependOn(&run_ghostty_tests.step);

    _ = try zx.init(b, app_exe, .{});
}
