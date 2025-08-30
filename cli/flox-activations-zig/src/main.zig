const std = @import("std");
const process = std.process;
const print = std.debug.print;
const Allocator = std.mem.Allocator;

const cli = @import("cli.zig");
const activations = @import("activations.zig");
const shell_gen = @import("shell_gen.zig");

pub const Error = error{
    InvalidArgs,
    FileNotFound,
    PermissionDenied,
    OutOfMemory,
    JsonParseError,
    InvalidJson,
    ProcessError,
    LockError,
    ActivationError,
};

pub fn main() !void {
    var gpa = std.heap.GeneralPurposeAllocator(.{}){};
    defer _ = gpa.deinit();
    const allocator = gpa.allocator();

    const args = try process.argsAlloc(allocator);
    defer process.argsFree(allocator, args);

    const args_const = @as([]const []const u8, args);
    const parsed_args = cli.parseArgs(allocator, args_const) catch |err| switch (err) {
        Error.InvalidArgs => {
            try cli.printHelp();
            process.exit(1);
        },
        else => return err,
    };
    defer parsed_args.deinit(allocator);

    try executeCommand(allocator, parsed_args);
}

fn executeCommand(allocator: Allocator, args: cli.ParsedArgs) !void {
    switch (args.command) {
        .StartOrAttach => |start_attach_args| {
            try cli.startOrAttach(allocator, start_attach_args);
        },
        .SetReady => |set_ready_args| {
            try cli.setReady(allocator, set_ready_args);
        },
        .Attach => |attach_args| {
            try cli.attach(allocator, attach_args);
        },
        .FixPaths => |fix_paths_args| {
            try cli.fixPaths(allocator, fix_paths_args);
        },
        .SetEnvDirs => |set_env_dirs_args| {
            try cli.setEnvDirs(allocator, set_env_dirs_args);
        },
        .ProfileScripts => |profile_scripts_args| {
            try cli.profileScripts(allocator, profile_scripts_args);
        },
        .PrependAndDedup => |prepend_dedup_args| {
            try cli.prependAndDedup(allocator, prepend_dedup_args);
        },
        .FixFpath => |fix_fpath_args| {
            try cli.fixFpath(allocator, fix_fpath_args);
        },
    }
}

test {
    std.testing.refAllDecls(@This());
}