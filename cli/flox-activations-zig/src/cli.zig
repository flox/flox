const std = @import("std");
const print = std.debug.print;
const Allocator = std.mem.Allocator;

const activations = @import("activations.zig");
const shell_gen = @import("shell_gen.zig");

pub const Error = @import("main.zig").Error;

pub const Command = union(enum) {
    StartOrAttach: StartOrAttachArgs,
    SetReady: SetReadyArgs,
    Attach: AttachArgs,
    FixPaths: FixPathsArgs,
    SetEnvDirs: SetEnvDirsArgs,
    ProfileScripts: ProfileScriptsArgs,
    PrependAndDedup: PrependAndDedupArgs,
    FixFpath: FixFpathArgs,
};

pub const StartOrAttachArgs = struct {
    pid: i32,
    flox_env: []const u8,
    store_path: []const u8,
    runtime_dir: []const u8,
};

pub const SetReadyArgs = struct {
    flox_env: []const u8,
    id: []const u8,
    runtime_dir: []const u8,
};

pub const AttachArgs = struct {
    pid: i32,
    flox_env: []const u8,
    id: []const u8,
    timeout_ms: ?u32,
    remove_pid: ?i32,
    runtime_dir: []const u8,
};

pub const FixPathsArgs = struct {
    flox_env_dirs: []const u8,
    path: []const u8,
    manpath: []const u8,
    shell: shell_gen.Shell,
};

pub const SetEnvDirsArgs = struct {
    flox_env: []const u8,
    env_dirs: []const u8,
    shell: shell_gen.Shell,
};

pub const ProfileScriptsArgs = struct {
    flox_env_dirs: []const u8,
    sourced_profile_scripts: []const u8,
    shell: shell_gen.Shell,
};

pub const PrependAndDedupArgs = struct {
    flox_env_dirs: []const u8,
    suffixes: ?[]const []const u8,
    pathlike_var: []const u8,
    prune: bool,
};

pub const FixFpathArgs = struct {
    flox_env_dirs: []const u8,
    fpath: []const u8,
};

pub const ParsedArgs = struct {
    command: Command,
    
    pub fn deinit(self: *const ParsedArgs, allocator: Allocator) void {
        _ = self;
        _ = allocator;
        // Memory cleanup is handled by the allocator
    }
};

const HELP_TEXT = 
    \\flox-activations - Monitors activation lifecycle to perform cleanup.
    \\
    \\USAGE:
    \\    flox-activations <SUBCOMMAND>
    \\
    \\SUBCOMMANDS:
    \\    start-or-attach    Start a new activation or attach to an existing one
    \\    set-ready          Set that the activation is ready to be attached to
    \\    attach             Attach to an existing activation
    \\    fix-paths          Print sourceable output fixing PATH and MANPATH for a shell
    \\    set-env-dirs       Print sourceable output setting FLOX_ENV_DIRS
    \\    profile-scripts    Print sourceable output that sources the user's profile scripts
    \\    prepend-and-dedup  Prepends and dedups environment dirs
    \\    fix-fpath          Print sourceable output fixing fpath/FPATH for zsh
    \\
;

pub fn printHelp() !void {
    try std.io.getStdErr().writer().writeAll(HELP_TEXT);
}

pub fn parseArgs(allocator: Allocator, args: []const []const u8) !ParsedArgs {
    if (args.len < 2) {
        return Error.InvalidArgs;
    }

    const subcommand = args[1];
    
    if (std.mem.eql(u8, subcommand, "start-or-attach")) {
        return ParsedArgs{ 
            .command = Command{ .StartOrAttach = try parseStartOrAttachArgs(allocator, args[2..]) }
        };
    } else if (std.mem.eql(u8, subcommand, "set-ready")) {
        return ParsedArgs{ 
            .command = Command{ .SetReady = try parseSetReadyArgs(allocator, args[2..]) }
        };
    } else if (std.mem.eql(u8, subcommand, "attach")) {
        return ParsedArgs{ 
            .command = Command{ .Attach = try parseAttachArgs(allocator, args[2..]) }
        };
    } else if (std.mem.eql(u8, subcommand, "fix-paths")) {
        return ParsedArgs{ 
            .command = Command{ .FixPaths = try parseFixPathsArgs(allocator, args[2..]) }
        };
    } else if (std.mem.eql(u8, subcommand, "set-env-dirs")) {
        return ParsedArgs{ 
            .command = Command{ .SetEnvDirs = try parseSetEnvDirsArgs(allocator, args[2..]) }
        };
    } else if (std.mem.eql(u8, subcommand, "profile-scripts")) {
        return ParsedArgs{ 
            .command = Command{ .ProfileScripts = try parseProfileScriptsArgs(allocator, args[2..]) }
        };
    } else if (std.mem.eql(u8, subcommand, "prepend-and-dedup")) {
        return ParsedArgs{ 
            .command = Command{ .PrependAndDedup = try parsePrependAndDedupArgs(allocator, args[2..]) }
        };
    } else if (std.mem.eql(u8, subcommand, "fix-fpath")) {
        return ParsedArgs{ 
            .command = Command{ .FixFpath = try parseFixFpathArgs(allocator, args[2..]) }
        };
    } else {
        return Error.InvalidArgs;
    }
}

fn parseStartOrAttachArgs(allocator: Allocator, args: []const []const u8) !StartOrAttachArgs {
    _ = allocator;
    
    var pid: ?i32 = null;
    var flox_env: ?[]const u8 = null;
    var store_path: ?[]const u8 = null;
    var runtime_dir: ?[]const u8 = null;
    
    var i: usize = 0;
    while (i < args.len) : (i += 1) {
        const arg = args[i];
        if (std.mem.eql(u8, arg, "--pid") or std.mem.eql(u8, arg, "-p")) {
            i += 1;
            if (i >= args.len) return Error.InvalidArgs;
            pid = std.fmt.parseInt(i32, args[i], 10) catch return Error.InvalidArgs;
        } else if (std.mem.eql(u8, arg, "--flox-env") or std.mem.eql(u8, arg, "-f")) {
            i += 1;
            if (i >= args.len) return Error.InvalidArgs;
            flox_env = args[i];
        } else if (std.mem.eql(u8, arg, "--store-path") or std.mem.eql(u8, arg, "-s")) {
            i += 1;
            if (i >= args.len) return Error.InvalidArgs;
            store_path = args[i];
        } else if (std.mem.eql(u8, arg, "--runtime-dir")) {
            i += 1;
            if (i >= args.len) return Error.InvalidArgs;
            runtime_dir = args[i];
        }
    }
    
    return StartOrAttachArgs{
        .pid = pid orelse return Error.InvalidArgs,
        .flox_env = flox_env orelse return Error.InvalidArgs,
        .store_path = store_path orelse return Error.InvalidArgs,
        .runtime_dir = runtime_dir orelse return Error.InvalidArgs,
    };
}

fn parseSetReadyArgs(allocator: Allocator, args: []const []const u8) !SetReadyArgs {
    _ = allocator;
    
    var flox_env: ?[]const u8 = null;
    var id: ?[]const u8 = null;
    var runtime_dir: ?[]const u8 = null;
    
    var i: usize = 0;
    while (i < args.len) : (i += 1) {
        const arg = args[i];
        if (std.mem.eql(u8, arg, "--flox-env") or std.mem.eql(u8, arg, "-f")) {
            i += 1;
            if (i >= args.len) return Error.InvalidArgs;
            flox_env = args[i];
        } else if (std.mem.eql(u8, arg, "--id") or std.mem.eql(u8, arg, "-i")) {
            i += 1;
            if (i >= args.len) return Error.InvalidArgs;
            id = args[i];
        } else if (std.mem.eql(u8, arg, "--runtime-dir")) {
            i += 1;
            if (i >= args.len) return Error.InvalidArgs;
            runtime_dir = args[i];
        }
    }
    
    return SetReadyArgs{
        .flox_env = flox_env orelse return Error.InvalidArgs,
        .id = id orelse return Error.InvalidArgs,
        .runtime_dir = runtime_dir orelse return Error.InvalidArgs,
    };
}

fn parseAttachArgs(allocator: Allocator, args: []const []const u8) !AttachArgs {
    _ = allocator;
    
    var pid: ?i32 = null;
    var flox_env: ?[]const u8 = null;
    var id: ?[]const u8 = null;
    var timeout_ms: ?u32 = null;
    var remove_pid: ?i32 = null;
    var runtime_dir: ?[]const u8 = null;
    
    var i: usize = 0;
    while (i < args.len) : (i += 1) {
        const arg = args[i];
        if (std.mem.eql(u8, arg, "--pid") or std.mem.eql(u8, arg, "-p")) {
            i += 1;
            if (i >= args.len) return Error.InvalidArgs;
            pid = std.fmt.parseInt(i32, args[i], 10) catch return Error.InvalidArgs;
        } else if (std.mem.eql(u8, arg, "--flox-env") or std.mem.eql(u8, arg, "-f")) {
            i += 1;
            if (i >= args.len) return Error.InvalidArgs;
            flox_env = args[i];
        } else if (std.mem.eql(u8, arg, "--id") or std.mem.eql(u8, arg, "-i")) {
            i += 1;
            if (i >= args.len) return Error.InvalidArgs;
            id = args[i];
        } else if (std.mem.eql(u8, arg, "--timeout-ms") or std.mem.eql(u8, arg, "-t")) {
            i += 1;
            if (i >= args.len) return Error.InvalidArgs;
            timeout_ms = std.fmt.parseInt(u32, args[i], 10) catch return Error.InvalidArgs;
        } else if (std.mem.eql(u8, arg, "--remove-pid") or std.mem.eql(u8, arg, "-r")) {
            i += 1;
            if (i >= args.len) return Error.InvalidArgs;
            remove_pid = std.fmt.parseInt(i32, args[i], 10) catch return Error.InvalidArgs;
        } else if (std.mem.eql(u8, arg, "--runtime-dir")) {
            i += 1;
            if (i >= args.len) return Error.InvalidArgs;
            runtime_dir = args[i];
        }
    }
    
    return AttachArgs{
        .pid = pid orelse return Error.InvalidArgs,
        .flox_env = flox_env orelse return Error.InvalidArgs,
        .id = id orelse return Error.InvalidArgs,
        .timeout_ms = timeout_ms,
        .remove_pid = remove_pid,
        .runtime_dir = runtime_dir orelse return Error.InvalidArgs,
    };
}

fn parseFixPathsArgs(allocator: Allocator, args: []const []const u8) !FixPathsArgs {
    _ = allocator;
    
    var flox_env_dirs: ?[]const u8 = null;
    var path: ?[]const u8 = null;
    var manpath: ?[]const u8 = null;
    var shell: shell_gen.Shell = .Bash;
    
    var i: usize = 0;
    while (i < args.len) : (i += 1) {
        const arg = args[i];
        if (std.mem.eql(u8, arg, "--flox-env-dirs")) {
            i += 1;
            if (i >= args.len) return Error.InvalidArgs;
            flox_env_dirs = args[i];
        } else if (std.mem.eql(u8, arg, "--path")) {
            i += 1;
            if (i >= args.len) return Error.InvalidArgs;
            path = args[i];
        } else if (std.mem.eql(u8, arg, "--manpath")) {
            i += 1;
            if (i >= args.len) return Error.InvalidArgs;
            manpath = args[i];
        } else if (std.mem.eql(u8, arg, "--shell")) {
            i += 1;
            if (i >= args.len) return Error.InvalidArgs;
            shell = shell_gen.parseShell(args[i]) orelse return Error.InvalidArgs;
        }
    }
    
    return FixPathsArgs{
        .flox_env_dirs = flox_env_dirs orelse return Error.InvalidArgs,
        .path = path orelse return Error.InvalidArgs,
        .manpath = manpath orelse return Error.InvalidArgs,
        .shell = shell,
    };
}

fn parseSetEnvDirsArgs(allocator: Allocator, args: []const []const u8) !SetEnvDirsArgs {
    _ = allocator;
    
    var flox_env: ?[]const u8 = null;
    var env_dirs: ?[]const u8 = null;
    var shell: shell_gen.Shell = .Bash;
    
    var i: usize = 0;
    while (i < args.len) : (i += 1) {
        const arg = args[i];
        if (std.mem.eql(u8, arg, "--flox-env")) {
            i += 1;
            if (i >= args.len) return Error.InvalidArgs;
            flox_env = args[i];
        } else if (std.mem.eql(u8, arg, "--env-dirs")) {
            i += 1;
            if (i >= args.len) return Error.InvalidArgs;
            env_dirs = args[i];
        } else if (std.mem.eql(u8, arg, "--shell")) {
            i += 1;
            if (i >= args.len) return Error.InvalidArgs;
            shell = shell_gen.parseShell(args[i]) orelse return Error.InvalidArgs;
        }
    }
    
    return SetEnvDirsArgs{
        .flox_env = flox_env orelse return Error.InvalidArgs,
        .env_dirs = env_dirs orelse return Error.InvalidArgs,
        .shell = shell,
    };
}

fn parseProfileScriptsArgs(allocator: Allocator, args: []const []const u8) !ProfileScriptsArgs {
    _ = allocator;
    
    var flox_env_dirs: ?[]const u8 = null;
    var sourced_profile_scripts: ?[]const u8 = null;
    var shell: shell_gen.Shell = .Bash;
    
    var i: usize = 0;
    while (i < args.len) : (i += 1) {
        const arg = args[i];
        if (std.mem.eql(u8, arg, "--flox-env-dirs")) {
            i += 1;
            if (i >= args.len) return Error.InvalidArgs;
            flox_env_dirs = args[i];
        } else if (std.mem.eql(u8, arg, "--sourced-profile-scripts")) {
            i += 1;
            if (i >= args.len) return Error.InvalidArgs;
            sourced_profile_scripts = args[i];
        } else if (std.mem.eql(u8, arg, "--shell")) {
            i += 1;
            if (i >= args.len) return Error.InvalidArgs;
            shell = shell_gen.parseShell(args[i]) orelse return Error.InvalidArgs;
        }
    }
    
    return ProfileScriptsArgs{
        .flox_env_dirs = flox_env_dirs orelse return Error.InvalidArgs,
        .sourced_profile_scripts = sourced_profile_scripts orelse return Error.InvalidArgs,
        .shell = shell,
    };
}

fn parsePrependAndDedupArgs(allocator: Allocator, args: []const []const u8) !PrependAndDedupArgs {
    _ = allocator;
    
    var flox_env_dirs: ?[]const u8 = null;
    var suffixes: ?[]const []const u8 = null;
    var pathlike_var: ?[]const u8 = null;
    var prune: bool = false;
    
    var i: usize = 0;
    while (i < args.len) : (i += 1) {
        const arg = args[i];
        if (std.mem.eql(u8, arg, "--flox-env-dirs")) {
            i += 1;
            if (i >= args.len) return Error.InvalidArgs;
            flox_env_dirs = args[i];
        } else if (std.mem.eql(u8, arg, "--suffixes")) {
            i += 1;
            if (i >= args.len) return Error.InvalidArgs;
            // For simplicity, treating suffixes as a single string that will be split
            // In real implementation, this should be parsed as multiple values
            suffixes = &[_][]const u8{args[i]};
        } else if (std.mem.eql(u8, arg, "--pathlike-var")) {
            i += 1;
            if (i >= args.len) return Error.InvalidArgs;
            pathlike_var = args[i];
        } else if (std.mem.eql(u8, arg, "--prune")) {
            prune = true;
        }
    }
    
    return PrependAndDedupArgs{
        .flox_env_dirs = flox_env_dirs orelse return Error.InvalidArgs,
        .suffixes = suffixes,
        .pathlike_var = pathlike_var orelse return Error.InvalidArgs,
        .prune = prune,
    };
}

fn parseFixFpathArgs(allocator: Allocator, args: []const []const u8) !FixFpathArgs {
    _ = allocator;
    
    var flox_env_dirs: ?[]const u8 = null;
    var fpath: ?[]const u8 = null;
    
    var i: usize = 0;
    while (i < args.len) : (i += 1) {
        const arg = args[i];
        if (std.mem.eql(u8, arg, "--flox-env-dirs")) {
            i += 1;
            if (i >= args.len) return Error.InvalidArgs;
            flox_env_dirs = args[i];
        } else if (std.mem.eql(u8, arg, "--fpath")) {
            i += 1;
            if (i >= args.len) return Error.InvalidArgs;
            fpath = args[i];
        }
    }
    
    return FixFpathArgs{
        .flox_env_dirs = flox_env_dirs orelse return Error.InvalidArgs,
        .fpath = fpath orelse return Error.InvalidArgs,
    };
}

// Implementation of command handlers
pub fn startOrAttach(allocator: Allocator, args: StartOrAttachArgs) !void {
    const result = try activations.startOrAttachImpl(allocator, args);
    defer result.deinit(allocator);
    
    print("_FLOX_ATTACH={}\n", .{result.attaching});
    print("_FLOX_ACTIVATION_STATE_DIR={s}\n", .{result.state_dir});
    print("_FLOX_ACTIVATION_ID={s}\n", .{result.activation_id});
}

pub fn setReady(allocator: Allocator, args: SetReadyArgs) !void {
    try activations.setReadyImpl(allocator, args);
}

pub fn attach(allocator: Allocator, args: AttachArgs) !void {
    try activations.attachImpl(allocator, args);
}

pub fn fixPaths(allocator: Allocator, args: FixPathsArgs) !void {
    const output = try shell_gen.fixPaths(allocator, args.flox_env_dirs, args.path, args.manpath, args.shell);
    defer allocator.free(output);
    print("{s}\n", .{output});
}

pub fn setEnvDirs(allocator: Allocator, args: SetEnvDirsArgs) !void {
    const output = try shell_gen.setEnvDirs(allocator, args.flox_env, args.env_dirs, args.shell);
    defer allocator.free(output);
    print("{s}\n", .{output});
}

pub fn profileScripts(allocator: Allocator, args: ProfileScriptsArgs) !void {
    const output = try shell_gen.profileScripts(allocator, args.flox_env_dirs, args.sourced_profile_scripts, args.shell);
    defer allocator.free(output);
    print("{s}\n", .{output});
}

pub fn prependAndDedup(allocator: Allocator, args: PrependAndDedupArgs) !void {
    const output = try shell_gen.prependAndDedup(allocator, args.flox_env_dirs, args.suffixes, args.pathlike_var, args.prune);
    defer allocator.free(output);
    print("{s}\n", .{output});
}

pub fn fixFpath(allocator: Allocator, args: FixFpathArgs) !void {
    const output = try shell_gen.fixFpath(allocator, args.flox_env_dirs, args.fpath);
    defer allocator.free(output);
    print("{s}\n", .{output});
}

// Test functions
test "parseArgs with start-or-attach" {
    const allocator = std.testing.allocator;
    const args = [_][]const u8{ "flox-activations", "start-or-attach", "--pid", "1234", "--flox-env", "/path", "--store-path", "/store", "--runtime-dir", "/runtime" };
    const result = try parseArgs(allocator, &args);
    defer result.deinit(allocator);
    
    switch (result.command) {
        .StartOrAttach => |start_attach| {
            try std.testing.expect(start_attach.pid == 1234);
            try std.testing.expect(std.mem.eql(u8, start_attach.flox_env, "/path"));
            try std.testing.expect(std.mem.eql(u8, start_attach.store_path, "/store"));
            try std.testing.expect(std.mem.eql(u8, start_attach.runtime_dir, "/runtime"));
        },
        else => try std.testing.expect(false),
    }
}

test "parseArgs invalid command" {
    const allocator = std.testing.allocator;
    const args = [_][]const u8{ "flox-activations", "invalid-command" };
    const result = parseArgs(allocator, &args);
    try std.testing.expectError(Error.InvalidArgs, result);
}