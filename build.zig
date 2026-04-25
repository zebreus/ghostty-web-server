const std = @import("std");

pub fn build(b: *std.Build) void {
    const target = b.standardTargetOptions(.{});
    const optimize = b.standardOptimizeOption(.{});

    const exe = b.addExecutable(.{
        .name = "ghostty-web-server",
        .root_source_file = b.path("src/server/main.zig"),
        .target = target,
        .optimize = optimize,
    });
    exe.linkLibC();
    exe.linkSystemLibrary("util");
    b.installArtifact(exe);

    const wasm_target = b.resolveTargetQuery(.{
        .cpu_arch = .wasm32,
        .os_tag = .freestanding,
        .abi = .none,
    });
    const client_wasm = b.addExecutable(.{
        .name = "client",
        .root_source_file = b.path("src/client/terminal.zig"),
        .target = wasm_target,
        .optimize = optimize,
    });
    client_wasm.entry = .disabled;
    client_wasm.rdynamic = true;

    const install_wasm = b.addInstallArtifact(client_wasm, .{ .dest_dir = .{ .override = .{ .custom = "bin" } } });
    b.getInstallStep().dependOn(&install_wasm.step);

    const install_client_js = b.addInstallFile(b.path("src/client/bootstrap.js"), "bin/client.js");
    b.getInstallStep().dependOn(&install_client_js.step);

    const run_cmd = b.addRunArtifact(exe);
    run_cmd.step.dependOn(b.getInstallStep());
    if (b.args) |args| run_cmd.addArgs(args);

    const run_step = b.step("run", "Run the server");
    run_step.dependOn(&run_cmd.step);
}
