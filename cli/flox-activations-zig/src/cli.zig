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
    \\    flox-activations [OPTIONS] <SUBCOMMAND>
    \\
    \\OPTIONS:
    \\    -h, --help       Print help information
    \\    -V, --version    Print version information
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
    \\Use 'flox-activations <SUBCOMMAND> --help' for more information on a subcommand.
    \\
;

const START_OR_ATTACH_HELP =
    \\Start a new activation or attach to an existing one.
    \\
    \\USAGE:
    \\    flox-activations start-or-attach [OPTIONS]
    \\
    \\OPTIONS:
    \\    -p, --pid <PID>                 The PID of the shell registering interest in the activation
    \\    -f, --flox-env <PATH>           The path to the activation symlink for the environment
    \\    -s, --store-path <PATH>         The store path of the rendered environment for this activation
    \\        --runtime-dir <PATH>        The path to the runtime directory keeping activation data
    \\    -h, --help                      Print help information
    \\
;

const SET_READY_HELP =
    \\Set that the activation is ready to be attached to.
    \\
    \\USAGE:
    \\    flox-activations set-ready [OPTIONS]
    \\
    \\OPTIONS:
    \\    -f, --flox-env <PATH>           The path to the activation symlink for the environment
    \\    -i, --id <ID>                   The ID for this particular activation of this environment
    \\        --runtime-dir <PATH>        The path to the runtime directory keeping activation data
    \\    -h, --help                      Print help information
    \\
;

const ATTACH_HELP =
    \\Attach to an existing activation.
    \\
    \\USAGE:
    \\    flox-activations attach [OPTIONS]
    \\
    \\OPTIONS:
    \\    -p, --pid <PID>                 The PID of the shell registering interest in the activation
    \\    -f, --flox-env <PATH>           The path to the activation symlink for the environment
    \\    -i, --id <ID>                   The ID for this particular activation of this environment
    \\    -t, --timeout-ms <TIME_MS>      How long to wait between termination of this PID and cleaning up its interest
    \\    -r, --remove-pid <PID>          Remove the specified PID when attaching to this activation
    \\        --runtime-dir <PATH>        The path to the runtime directory keeping activation data
    \\    -h, --help                      Print help information
    \\
    \\NOTE: Exactly one of --timeout-ms or --remove-pid must be specified.
    \\
;

const FIX_PATHS_HELP =
    \\Print sourceable output fixing PATH and MANPATH for a shell.
    \\
    \\USAGE:
    \\    flox-activations fix-paths [OPTIONS]
    \\
    \\OPTIONS:
    \\        --flox-env-dirs <DIRS>      Colon-separated list of flox environment directories
    \\        --path <PATH>               Current PATH environment variable
    \\        --manpath <MANPATH>         Current MANPATH environment variable
    \\        --shell <SHELL>             Target shell (bash, zsh, fish, tcsh) [default: bash]
    \\    -h, --help                      Print help information
    \\
;

const SET_ENV_DIRS_HELP =
    \\Print sourceable output setting FLOX_ENV_DIRS.
    \\
    \\USAGE:
    \\    flox-activations set-env-dirs [OPTIONS]
    \\
    \\OPTIONS:
    \\        --flox-env <PATH>           The path to the flox environment
    \\        --env-dirs <DIRS>           Existing FLOX_ENV_DIRS environment variable
    \\        --shell <SHELL>             Target shell (bash, zsh, fish, tcsh) [default: bash]
    \\    -h, --help                      Print help information
    \\
;

const PROFILE_SCRIPTS_HELP =
    \\Print sourceable output that sources the user's profile scripts.
    \\
    \\USAGE:
    \\    flox-activations profile-scripts [OPTIONS]
    \\
    \\OPTIONS:
    \\        --flox-env-dirs <DIRS>           Colon-separated list of flox environment directories
    \\        --sourced-profile-scripts <DIRS> Already sourced profile script directories
    \\        --shell <SHELL>                  Target shell (bash, zsh, fish, tcsh) [default: bash]
    \\    -h, --help                           Print help information
    \\
;

const PREPEND_AND_DEDUP_HELP =
    \\Prepends and dedups environment dirs, optionally pruning directories that aren't from environments.
    \\
    \\USAGE:
    \\    flox-activations prepend-and-dedup [OPTIONS]
    \\
    \\OPTIONS:
    \\        --flox-env-dirs <DIRS>      Colon-separated list of flox environment directories
    \\        --suffixes <SUFFIXES>       Path suffixes to append to each directory
    \\        --pathlike-var <VAR>        Current value of the PATH-like variable
    \\        --prune                     Prune directories that aren't from environments
    \\    -h, --help                      Print help information
    \\
;

const FIX_FPATH_HELP =
    \\Print sourceable output fixing fpath/FPATH for zsh.
    \\
    \\USAGE:
    \\    flox-activations fix-fpath [OPTIONS]
    \\
    \\OPTIONS:
    \\        --flox-env-dirs <DIRS>      Colon-separated list of flox environment directories
    \\        --fpath <FPATH>             Current fpath/FPATH environment variable
    \\    -h, --help                      Print help information
    \\
;

pub fn printHelp() !void {
    try std.io.getStdErr().writer().writeAll(HELP_TEXT);
}

pub fn printError(comptime fmt: []const u8, args: anytype) !void {
    try std.io.getStdErr().writer().print(fmt, args);
}

pub fn printSubcommandHelp(subcommand: []const u8) !void {
    const help_text = if (std.mem.eql(u8, subcommand, "start-or-attach"))
        START_OR_ATTACH_HELP
    else if (std.mem.eql(u8, subcommand, "set-ready"))
        SET_READY_HELP
    else if (std.mem.eql(u8, subcommand, "attach"))
        ATTACH_HELP
    else if (std.mem.eql(u8, subcommand, "fix-paths"))
        FIX_PATHS_HELP
    else if (std.mem.eql(u8, subcommand, "set-env-dirs"))
        SET_ENV_DIRS_HELP
    else if (std.mem.eql(u8, subcommand, "profile-scripts"))
        PROFILE_SCRIPTS_HELP
    else if (std.mem.eql(u8, subcommand, "prepend-and-dedup"))
        PREPEND_AND_DEDUP_HELP
    else if (std.mem.eql(u8, subcommand, "fix-fpath"))
        FIX_FPATH_HELP
    else
        HELP_TEXT;
    
    try std.io.getStdErr().writer().writeAll(help_text);
}

pub fn parseArgs(allocator: Allocator, args: []const []const u8) !ParsedArgs {
    if (args.len < 2) {
        try printError("Error: No subcommand provided.\n\n", .{});
        try printHelp();
        return Error.InvalidArgs;
    }

    const subcommand = args[1];
    
    // Handle global help and version flags
    if (std.mem.eql(u8, subcommand, "--help") or std.mem.eql(u8, subcommand, "-h")) {
        try printHelp();
        std.process.exit(0);
    }
    
    if (std.mem.eql(u8, subcommand, "--version") or std.mem.eql(u8, subcommand, "-V")) {
        try std.io.getStdOut().writer().writeAll("flox-activations 0.0.0\n");
        std.process.exit(0);
    }
    
    if (std.mem.eql(u8, subcommand, "start-or-attach")) {
        return ParsedArgs{ 
            .command = Command{ .StartOrAttach = parseStartOrAttachArgs(allocator, args[2..]) catch |err| {
                try printError("Error parsing start-or-attach command arguments.\n\n", .{});
                try printSubcommandHelp("start-or-attach");
                return err;
            }}
        };
    } else if (std.mem.eql(u8, subcommand, "set-ready")) {
        return ParsedArgs{ 
            .command = Command{ .SetReady = parseSetReadyArgs(allocator, args[2..]) catch |err| {
                try printError("Error parsing set-ready command arguments.\n\n", .{});
                try printSubcommandHelp("set-ready");
                return err;
            }}
        };
    } else if (std.mem.eql(u8, subcommand, "attach")) {
        return ParsedArgs{ 
            .command = Command{ .Attach = parseAttachArgs(allocator, args[2..]) catch |err| {
                try printError("Error parsing attach command arguments.\n\n", .{});
                try printSubcommandHelp("attach");
                return err;
            }}
        };
    } else if (std.mem.eql(u8, subcommand, "fix-paths")) {
        return ParsedArgs{ 
            .command = Command{ .FixPaths = parseFixPathsArgs(allocator, args[2..]) catch |err| {
                try printError("Error parsing fix-paths command arguments.\n\n", .{});
                try printSubcommandHelp("fix-paths");
                return err;
            }}
        };
    } else if (std.mem.eql(u8, subcommand, "set-env-dirs")) {
        return ParsedArgs{ 
            .command = Command{ .SetEnvDirs = parseSetEnvDirsArgs(allocator, args[2..]) catch |err| {
                try printError("Error parsing set-env-dirs command arguments.\n\n", .{});
                try printSubcommandHelp("set-env-dirs");
                return err;
            }}
        };
    } else if (std.mem.eql(u8, subcommand, "profile-scripts")) {
        return ParsedArgs{ 
            .command = Command{ .ProfileScripts = parseProfileScriptsArgs(allocator, args[2..]) catch |err| {
                try printError("Error parsing profile-scripts command arguments.\n\n", .{});
                try printSubcommandHelp("profile-scripts");
                return err;
            }}
        };
    } else if (std.mem.eql(u8, subcommand, "prepend-and-dedup")) {
        return ParsedArgs{ 
            .command = Command{ .PrependAndDedup = parsePrependAndDedupArgs(allocator, args[2..]) catch |err| {
                try printError("Error parsing prepend-and-dedup command arguments.\n\n", .{});
                try printSubcommandHelp("prepend-and-dedup");
                return err;
            }}
        };
    } else if (std.mem.eql(u8, subcommand, "fix-fpath")) {
        return ParsedArgs{ 
            .command = Command{ .FixFpath = parseFixFpathArgs(allocator, args[2..]) catch |err| {
                try printError("Error parsing fix-fpath command arguments.\n\n", .{});
                try printSubcommandHelp("fix-fpath");
                return err;
            }}
        };
    } else {
        try printError("Error: Unknown subcommand '{s}'.\n\n", .{subcommand});
        try printHelp();
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
        if (std.mem.eql(u8, arg, "--help") or std.mem.eql(u8, arg, "-h")) {
            try printSubcommandHelp("start-or-attach");
            std.process.exit(0);
        } else if (std.mem.eql(u8, arg, "--pid") or std.mem.eql(u8, arg, "-p")) {
            i += 1;
            if (i >= args.len) {
                try printError("Error: --pid requires a value.\n\n", .{});
                return Error.InvalidArgs;
            }
            pid = std.fmt.parseInt(i32, args[i], 10) catch {
                try printError("Error: Invalid PID '{s}'. Must be a valid integer.\n\n", .{args[i]});
                return Error.InvalidArgs;
            };
        } else if (std.mem.eql(u8, arg, "--flox-env") or std.mem.eql(u8, arg, "-f")) {
            i += 1;
            if (i >= args.len) {
                try printError("Error: --flox-env requires a value.\n\n", .{});
                return Error.InvalidArgs;
            }
            flox_env = args[i];
        } else if (std.mem.eql(u8, arg, "--store-path") or std.mem.eql(u8, arg, "-s")) {
            i += 1;
            if (i >= args.len) {
                try printError("Error: --store-path requires a value.\n\n", .{});
                return Error.InvalidArgs;
            }
            store_path = args[i];
        } else if (std.mem.eql(u8, arg, "--runtime-dir")) {
            i += 1;
            if (i >= args.len) {
                try printError("Error: --runtime-dir requires a value.\n\n", .{});
                return Error.InvalidArgs;
            }
            runtime_dir = args[i];
        } else {
            try printError("Error: Unknown argument '{s}' for start-or-attach command.\n\n", .{arg});
            return Error.InvalidArgs;
        }
    }
    
    if (pid == null) {
        try printError("Error: Missing required argument --pid.\n\n", .{});
        return Error.InvalidArgs;
    }
    if (flox_env == null) {
        try printError("Error: Missing required argument --flox-env.\n\n", .{});
        return Error.InvalidArgs;
    }
    if (store_path == null) {
        try printError("Error: Missing required argument --store-path.\n\n", .{});
        return Error.InvalidArgs;
    }
    if (runtime_dir == null) {
        try printError("Error: Missing required argument --runtime-dir.\n\n", .{});
        return Error.InvalidArgs;
    }
    
    return StartOrAttachArgs{
        .pid = pid.?,
        .flox_env = flox_env.?,
        .store_path = store_path.?,
        .runtime_dir = runtime_dir.?,
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
        if (std.mem.eql(u8, arg, "--help") or std.mem.eql(u8, arg, "-h")) {
            try printSubcommandHelp("set-ready");
            std.process.exit(0);
        } else if (std.mem.eql(u8, arg, "--flox-env") or std.mem.eql(u8, arg, "-f")) {
            i += 1;
            if (i >= args.len) {
                try printError("Error: --flox-env requires a value.\n\n", .{});
                return Error.InvalidArgs;
            }
            flox_env = args[i];
        } else if (std.mem.eql(u8, arg, "--id") or std.mem.eql(u8, arg, "-i")) {
            i += 1;
            if (i >= args.len) {
                try printError("Error: --id requires a value.\n\n", .{});
                return Error.InvalidArgs;
            }
            id = args[i];
        } else if (std.mem.eql(u8, arg, "--runtime-dir")) {
            i += 1;
            if (i >= args.len) {
                try printError("Error: --runtime-dir requires a value.\n\n", .{});
                return Error.InvalidArgs;
            }
            runtime_dir = args[i];
        } else {
            try printError("Error: Unknown argument '{s}' for set-ready command.\n\n", .{arg});
            return Error.InvalidArgs;
        }
    }
    
    if (flox_env == null) {
        try printError("Error: Missing required argument --flox-env.\n\n", .{});
        return Error.InvalidArgs;
    }
    if (id == null) {
        try printError("Error: Missing required argument --id.\n\n", .{});
        return Error.InvalidArgs;
    }
    if (runtime_dir == null) {
        try printError("Error: Missing required argument --runtime-dir.\n\n", .{});
        return Error.InvalidArgs;
    }
    
    return SetReadyArgs{
        .flox_env = flox_env.?,
        .id = id.?,
        .runtime_dir = runtime_dir.?,
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
        if (std.mem.eql(u8, arg, "--help") or std.mem.eql(u8, arg, "-h")) {
            try printSubcommandHelp("attach");
            std.process.exit(0);
        } else if (std.mem.eql(u8, arg, "--pid") or std.mem.eql(u8, arg, "-p")) {
            i += 1;
            if (i >= args.len) {
                try printError("Error: --pid requires a value.\n\n", .{});
                return Error.InvalidArgs;
            }
            pid = std.fmt.parseInt(i32, args[i], 10) catch {
                try printError("Error: Invalid PID '{s}'. Must be a valid integer.\n\n", .{args[i]});
                return Error.InvalidArgs;
            };
        } else if (std.mem.eql(u8, arg, "--flox-env") or std.mem.eql(u8, arg, "-f")) {
            i += 1;
            if (i >= args.len) {
                try printError("Error: --flox-env requires a value.\n\n", .{});
                return Error.InvalidArgs;
            }
            flox_env = args[i];
        } else if (std.mem.eql(u8, arg, "--id") or std.mem.eql(u8, arg, "-i")) {
            i += 1;
            if (i >= args.len) {
                try printError("Error: --id requires a value.\n\n", .{});
                return Error.InvalidArgs;
            }
            id = args[i];
        } else if (std.mem.eql(u8, arg, "--timeout-ms") or std.mem.eql(u8, arg, "-t")) {
            i += 1;
            if (i >= args.len) {
                try printError("Error: --timeout-ms requires a value.\n\n", .{});
                return Error.InvalidArgs;
            }
            timeout_ms = std.fmt.parseInt(u32, args[i], 10) catch {
                try printError("Error: Invalid timeout '{s}'. Must be a valid integer.\n\n", .{args[i]});
                return Error.InvalidArgs;
            };
        } else if (std.mem.eql(u8, arg, "--remove-pid") or std.mem.eql(u8, arg, "-r")) {
            i += 1;
            if (i >= args.len) {
                try printError("Error: --remove-pid requires a value.\n\n", .{});
                return Error.InvalidArgs;
            }
            remove_pid = std.fmt.parseInt(i32, args[i], 10) catch {
                try printError("Error: Invalid PID '{s}'. Must be a valid integer.\n\n", .{args[i]});
                return Error.InvalidArgs;
            };
        } else if (std.mem.eql(u8, arg, "--runtime-dir")) {
            i += 1;
            if (i >= args.len) {
                try printError("Error: --runtime-dir requires a value.\n\n", .{});
                return Error.InvalidArgs;
            }
            runtime_dir = args[i];
        } else {
            try printError("Error: Unknown argument '{s}' for attach command.\n\n", .{arg});
            return Error.InvalidArgs;
        }
    }
    
    // Validate required arguments
    if (pid == null) {
        try printError("Error: Missing required argument --pid.\n\n", .{});
        return Error.InvalidArgs;
    }
    if (flox_env == null) {
        try printError("Error: Missing required argument --flox-env.\n\n", .{});
        return Error.InvalidArgs;
    }
    if (id == null) {
        try printError("Error: Missing required argument --id.\n\n", .{});
        return Error.InvalidArgs;
    }
    if (runtime_dir == null) {
        try printError("Error: Missing required argument --runtime-dir.\n\n", .{});
        return Error.InvalidArgs;
    }
    
    // Validate exclusive group: exactly one of timeout_ms or remove_pid must be specified
    if (timeout_ms == null and remove_pid == null) {
        try printError("Error: Exactly one of --timeout-ms or --remove-pid must be specified.\n\n", .{});
        return Error.InvalidArgs;
    }
    if (timeout_ms != null and remove_pid != null) {
        try printError("Error: Cannot specify both --timeout-ms and --remove-pid. Choose one.\n\n", .{});
        return Error.InvalidArgs;
    }
    
    return AttachArgs{
        .pid = pid.?,
        .flox_env = flox_env.?,
        .id = id.?,
        .timeout_ms = timeout_ms,
        .remove_pid = remove_pid,
        .runtime_dir = runtime_dir.?,
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
        if (std.mem.eql(u8, arg, "--help") or std.mem.eql(u8, arg, "-h")) {
            try printSubcommandHelp("fix-paths");
            std.process.exit(0);
        } else if (std.mem.eql(u8, arg, "--flox-env-dirs")) {
            i += 1;
            if (i >= args.len) {
                try printError("Error: --flox-env-dirs requires a value.\n\n", .{});
                return Error.InvalidArgs;
            }
            flox_env_dirs = args[i];
        } else if (std.mem.eql(u8, arg, "--path")) {
            i += 1;
            if (i >= args.len) {
                try printError("Error: --path requires a value.\n\n", .{});
                return Error.InvalidArgs;
            }
            path = args[i];
        } else if (std.mem.eql(u8, arg, "--manpath")) {
            i += 1;
            if (i >= args.len) {
                try printError("Error: --manpath requires a value.\n\n", .{});
                return Error.InvalidArgs;
            }
            manpath = args[i];
        } else if (std.mem.eql(u8, arg, "--shell")) {
            i += 1;
            if (i >= args.len) {
                try printError("Error: --shell requires a value.\n\n", .{});
                return Error.InvalidArgs;
            }
            shell = shell_gen.parseShell(args[i]) orelse {
                try printError("Error: Invalid shell '{s}'. Must be one of: bash, zsh, fish, tcsh.\n\n", .{args[i]});
                return Error.InvalidArgs;
            };
        } else {
            try printError("Error: Unknown argument '{s}' for fix-paths command.\n\n", .{arg});
            return Error.InvalidArgs;
        }
    }
    
    if (flox_env_dirs == null) {
        try printError("Error: Missing required argument --flox-env-dirs.\n\n", .{});
        return Error.InvalidArgs;
    }
    if (path == null) {
        try printError("Error: Missing required argument --path.\n\n", .{});
        return Error.InvalidArgs;
    }
    if (manpath == null) {
        try printError("Error: Missing required argument --manpath.\n\n", .{});
        return Error.InvalidArgs;
    }
    
    return FixPathsArgs{
        .flox_env_dirs = flox_env_dirs.?,
        .path = path.?,
        .manpath = manpath.?,
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
        if (std.mem.eql(u8, arg, "--help") or std.mem.eql(u8, arg, "-h")) {
            try printSubcommandHelp("set-env-dirs");
            std.process.exit(0);
        } else if (std.mem.eql(u8, arg, "--flox-env")) {
            i += 1;
            if (i >= args.len) {
                try printError("Error: --flox-env requires a value.\n\n", .{});
                return Error.InvalidArgs;
            }
            flox_env = args[i];
        } else if (std.mem.eql(u8, arg, "--env-dirs")) {
            i += 1;
            if (i >= args.len) {
                try printError("Error: --env-dirs requires a value.\n\n", .{});
                return Error.InvalidArgs;
            }
            env_dirs = args[i];
        } else if (std.mem.eql(u8, arg, "--shell")) {
            i += 1;
            if (i >= args.len) {
                try printError("Error: --shell requires a value.\n\n", .{});
                return Error.InvalidArgs;
            }
            shell = shell_gen.parseShell(args[i]) orelse {
                try printError("Error: Invalid shell '{s}'. Must be one of: bash, zsh, fish, tcsh.\n\n", .{args[i]});
                return Error.InvalidArgs;
            };
        } else {
            try printError("Error: Unknown argument '{s}' for set-env-dirs command.\n\n", .{arg});
            return Error.InvalidArgs;
        }
    }
    
    if (flox_env == null) {
        try printError("Error: Missing required argument --flox-env.\n\n", .{});
        return Error.InvalidArgs;
    }
    if (env_dirs == null) {
        try printError("Error: Missing required argument --env-dirs.\n\n", .{});
        return Error.InvalidArgs;
    }
    
    return SetEnvDirsArgs{
        .flox_env = flox_env.?,
        .env_dirs = env_dirs.?,
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
        if (std.mem.eql(u8, arg, "--help") or std.mem.eql(u8, arg, "-h")) {
            try printSubcommandHelp("profile-scripts");
            std.process.exit(0);
        } else if (std.mem.eql(u8, arg, "--flox-env-dirs")) {
            i += 1;
            if (i >= args.len) {
                try printError("Error: --flox-env-dirs requires a value.\n\n", .{});
                return Error.InvalidArgs;
            }
            flox_env_dirs = args[i];
        } else if (std.mem.eql(u8, arg, "--sourced-profile-scripts")) {
            i += 1;
            if (i >= args.len) {
                try printError("Error: --sourced-profile-scripts requires a value.\n\n", .{});
                return Error.InvalidArgs;
            }
            sourced_profile_scripts = args[i];
        } else if (std.mem.eql(u8, arg, "--shell")) {
            i += 1;
            if (i >= args.len) {
                try printError("Error: --shell requires a value.\n\n", .{});
                return Error.InvalidArgs;
            }
            shell = shell_gen.parseShell(args[i]) orelse {
                try printError("Error: Invalid shell '{s}'. Must be one of: bash, zsh, fish, tcsh.\n\n", .{args[i]});
                return Error.InvalidArgs;
            };
        } else {
            try printError("Error: Unknown argument '{s}' for profile-scripts command.\n\n", .{arg});
            return Error.InvalidArgs;
        }
    }
    
    if (flox_env_dirs == null) {
        try printError("Error: Missing required argument --flox-env-dirs.\n\n", .{});
        return Error.InvalidArgs;
    }
    if (sourced_profile_scripts == null) {
        try printError("Error: Missing required argument --sourced-profile-scripts.\n\n", .{});
        return Error.InvalidArgs;
    }
    
    return ProfileScriptsArgs{
        .flox_env_dirs = flox_env_dirs.?,
        .sourced_profile_scripts = sourced_profile_scripts.?,
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
        if (std.mem.eql(u8, arg, "--help") or std.mem.eql(u8, arg, "-h")) {
            try printSubcommandHelp("prepend-and-dedup");
            std.process.exit(0);
        } else if (std.mem.eql(u8, arg, "--flox-env-dirs")) {
            i += 1;
            if (i >= args.len) {
                try printError("Error: --flox-env-dirs requires a value.\n\n", .{});
                return Error.InvalidArgs;
            }
            flox_env_dirs = args[i];
        } else if (std.mem.eql(u8, arg, "--suffixes")) {
            i += 1;
            if (i >= args.len) {
                try printError("Error: --suffixes requires a value.\n\n", .{});
                return Error.InvalidArgs;
            }
            // For simplicity, treating suffixes as a single string that will be split
            // In real implementation, this should be parsed as multiple values
            suffixes = &[_][]const u8{args[i]};
        } else if (std.mem.eql(u8, arg, "--pathlike-var")) {
            i += 1;
            if (i >= args.len) {
                try printError("Error: --pathlike-var requires a value.\n\n", .{});
                return Error.InvalidArgs;
            }
            pathlike_var = args[i];
        } else if (std.mem.eql(u8, arg, "--prune")) {
            prune = true;
        } else {
            try printError("Error: Unknown argument '{s}' for prepend-and-dedup command.\n\n", .{arg});
            return Error.InvalidArgs;
        }
    }
    
    if (flox_env_dirs == null) {
        try printError("Error: Missing required argument --flox-env-dirs.\n\n", .{});
        return Error.InvalidArgs;
    }
    if (pathlike_var == null) {
        try printError("Error: Missing required argument --pathlike-var.\n\n", .{});
        return Error.InvalidArgs;
    }
    
    return PrependAndDedupArgs{
        .flox_env_dirs = flox_env_dirs.?,
        .suffixes = suffixes,
        .pathlike_var = pathlike_var.?,
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
        if (std.mem.eql(u8, arg, "--help") or std.mem.eql(u8, arg, "-h")) {
            try printSubcommandHelp("fix-fpath");
            std.process.exit(0);
        } else if (std.mem.eql(u8, arg, "--flox-env-dirs")) {
            i += 1;
            if (i >= args.len) {
                try printError("Error: --flox-env-dirs requires a value.\n\n", .{});
                return Error.InvalidArgs;
            }
            flox_env_dirs = args[i];
        } else if (std.mem.eql(u8, arg, "--fpath")) {
            i += 1;
            if (i >= args.len) {
                try printError("Error: --fpath requires a value.\n\n", .{});
                return Error.InvalidArgs;
            }
            fpath = args[i];
        } else {
            try printError("Error: Unknown argument '{s}' for fix-fpath command.\n\n", .{arg});
            return Error.InvalidArgs;
        }
    }
    
    if (flox_env_dirs == null) {
        try printError("Error: Missing required argument --flox-env-dirs.\n\n", .{});
        return Error.InvalidArgs;
    }
    if (fpath == null) {
        try printError("Error: Missing required argument --fpath.\n\n", .{});
        return Error.InvalidArgs;
    }
    
    return FixFpathArgs{
        .flox_env_dirs = flox_env_dirs.?,
        .fpath = fpath.?,
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