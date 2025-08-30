const std = @import("std");
const testing = std.testing;
const Allocator = std.mem.Allocator;

const cli = @import("cli.zig");
const activations = @import("activations.zig");
const shell_gen = @import("shell_gen.zig");

// Test temporary directory helper
const TempDir = struct {
    dir: std.fs.Dir,
    path: []u8,
    allocator: Allocator,
    
    pub fn init(allocator: Allocator) !TempDir {
        var tmp_dir_obj = std.fs.cwd().openDir("/tmp", .{}) catch |err| switch (err) {
            error.FileNotFound => try std.fs.cwd().makeOpenPath("/tmp", .{}),
            else => return err,
        };
        defer tmp_dir_obj.close();
        
        const path = try std.fmt.allocPrint(allocator, "/tmp/flox-test-{}", .{std.time.milliTimestamp()});
        try std.fs.cwd().makeDir(path);
        const dir = try std.fs.cwd().openDir(path, .{});
        
        return TempDir{
            .dir = dir,
            .path = path,
            .allocator = allocator,
        };
    }
    
    pub fn deinit(self: *TempDir) void {
        self.dir.close();
        std.fs.cwd().deleteTree(self.path) catch {};
        self.allocator.free(self.path);
    }
    
    pub fn pathAlloc(self: *const TempDir, allocator: Allocator, sub_path: []const u8) ![]u8 {
        return try std.fmt.allocPrint(allocator, "{s}/{s}", .{ self.path, sub_path });
    }
};

// Test helpers for creating test activations
fn createTestActivations(allocator: Allocator, runtime_dir: []const u8, flox_env: []const u8, store_path: []const u8, pid: i32) ![]u8 {
    const activations_path = try activations.getActivationsJsonPath(allocator, runtime_dir, flox_env);
    defer allocator.free(activations_path);
    
    var activations_data = activations.Activations.init(allocator);
    defer activations_data.deinit(allocator);
    
    const activation = try activations_data.createActivation(allocator, store_path, pid);
    try activations.writeActivationsJson(allocator, &activations_data, activations_path);
    
    return try allocator.dupe(u8, activation.id);
}

// CLI parsing tests - ported from Rust cli tests
test "CLI: parseArgs with start-or-attach command" {
    const allocator = testing.allocator;
    const args = [_][]const u8{ 
        "flox-activations", 
        "start-or-attach", 
        "--pid", "1234", 
        "--flox-env", "/path/to/env", 
        "--store-path", "/nix/store/path", 
        "--runtime-dir", "/runtime" 
    };
    
    const result = try cli.parseArgsNoSideEffects(allocator, &args);
    defer result.deinit(allocator);
    
    switch (result.command) {
        .StartOrAttach => |start_attach| {
            try testing.expect(start_attach.pid == 1234);
            try testing.expect(std.mem.eql(u8, start_attach.flox_env, "/path/to/env"));
            try testing.expect(std.mem.eql(u8, start_attach.store_path, "/nix/store/path"));
            try testing.expect(std.mem.eql(u8, start_attach.runtime_dir, "/runtime"));
        },
        else => try testing.expect(false),
    }
}

test "CLI: parseArgs with set-ready command" {
    const allocator = testing.allocator;
    const args = [_][]const u8{ 
        "flox-activations", 
        "set-ready", 
        "--flox-env", "/path/to/env", 
        "--id", "12345", 
        "--runtime-dir", "/runtime" 
    };
    
    const result = try cli.parseArgsNoSideEffects(allocator, &args);
    defer result.deinit(allocator);
    
    switch (result.command) {
        .SetReady => |set_ready| {
            try testing.expect(std.mem.eql(u8, set_ready.flox_env, "/path/to/env"));
            try testing.expect(std.mem.eql(u8, set_ready.id, "12345"));
            try testing.expect(std.mem.eql(u8, set_ready.runtime_dir, "/runtime"));
        },
        else => try testing.expect(false),
    }
}

test "CLI: parseArgs with attach command" {
    const allocator = testing.allocator;
    const args = [_][]const u8{ 
        "flox-activations", 
        "attach", 
        "--pid", "5678", 
        "--flox-env", "/path/to/env", 
        "--id", "12345", 
        "--timeout-ms", "1000",
        "--runtime-dir", "/runtime" 
    };
    
    const result = try cli.parseArgsNoSideEffects(allocator, &args);
    defer result.deinit(allocator);
    
    switch (result.command) {
        .Attach => |attach| {
            try testing.expect(attach.pid == 5678);
            try testing.expect(std.mem.eql(u8, attach.flox_env, "/path/to/env"));
            try testing.expect(std.mem.eql(u8, attach.id, "12345"));
            try testing.expect(attach.timeout_ms.? == 1000);
            try testing.expect(std.mem.eql(u8, attach.runtime_dir, "/runtime"));
        },
        else => try testing.expect(false),
    }
}

test "CLI: parseArgs invalid command returns error" {
    const allocator = testing.allocator;
    const args = [_][]const u8{ "flox-activations", "invalid-command" };
    const result = cli.parseArgsNoSideEffects(allocator, &args);
    try testing.expectError(cli.Error.InvalidArgs, result);
}

test "CLI: parseArgs missing required arguments" {
    const allocator = testing.allocator;
    const args = [_][]const u8{ "flox-activations", "start-or-attach", "--pid", "1234" };
    const result = cli.parseArgsNoSideEffects(allocator, &args);
    try testing.expectError(cli.Error.InvalidArgs, result);
}

// Activations tests - ported from Rust start_or_attach tests
test "StartOrAttach: attach if activation exists" {
    const allocator = testing.allocator;
    var temp_dir = try TempDir.init(allocator);
    defer temp_dir.deinit();
    
    const flox_env = "/path/to/floxenv";
    const store_path = "/store/path";
    const pid = 1234;
    
    // Create existing activation
    const activation_id = try createTestActivations(allocator, temp_dir.path, flox_env, store_path, pid);
    defer allocator.free(activation_id);
    
    const args = cli.StartOrAttachArgs{
        .pid = pid,
        .flox_env = flox_env,
        .store_path = store_path,
        .runtime_dir = temp_dir.path,
    };
    
    const result = try activations.startOrAttachImpl(allocator, args);
    defer result.deinit(allocator);
    
    try testing.expect(result.attaching == true);
    try testing.expect(std.mem.indexOf(u8, result.state_dir, activation_id) != null);
    try testing.expect(std.mem.eql(u8, result.activation_id, activation_id));
}

test "StartOrAttach: start if activation does not exist" {
    const allocator = testing.allocator;
    var temp_dir = try TempDir.init(allocator);
    defer temp_dir.deinit();
    
    const flox_env = "/path/to/floxenv";
    const store_path = "/store/path";
    const pid = 1234;
    
    const args = cli.StartOrAttachArgs{
        .pid = pid,
        .flox_env = flox_env,
        .store_path = store_path,
        .runtime_dir = temp_dir.path,
    };
    
    const result = try activations.startOrAttachImpl(allocator, args);
    defer result.deinit(allocator);
    
    try testing.expect(result.attaching == false);
    try testing.expect(result.state_dir.len > 0);
    try testing.expect(result.activation_id.len > 0);
}

// Attach tests - ported from Rust attach tests
test "Attach: attach to id with new pid" {
    const allocator = testing.allocator;
    var temp_dir = try TempDir.init(allocator);
    defer temp_dir.deinit();
    
    const flox_env = "/path/to/floxenv";
    const store_path = "/store/path";
    const initial_pid = 1234;
    const new_pid = 5678;
    
    // Create activation
    const activation_id = try createTestActivations(allocator, temp_dir.path, flox_env, store_path, initial_pid);
    defer allocator.free(activation_id);
    
    const args = cli.AttachArgs{
        .pid = new_pid,
        .flox_env = flox_env,
        .id = activation_id,
        .timeout_ms = 1000,
        .remove_pid = null,
        .runtime_dir = temp_dir.path,
    };
    
    try activations.attachImpl(allocator, args);
    // If no error, the attach was successful
}

test "Attach: attach to id with replace pid" {
    const allocator = testing.allocator;
    var temp_dir = try TempDir.init(allocator);
    defer temp_dir.deinit();
    
    const flox_env = "/path/to/floxenv";
    const store_path = "/store/path";
    const old_pid = 1234;
    const new_pid = 5678;
    
    // Create activation
    const activation_id = try createTestActivations(allocator, temp_dir.path, flox_env, store_path, old_pid);
    defer allocator.free(activation_id);
    
    const args = cli.AttachArgs{
        .pid = new_pid,
        .flox_env = flox_env,
        .id = activation_id,
        .timeout_ms = null,
        .remove_pid = old_pid,
        .runtime_dir = temp_dir.path,
    };
    
    try activations.attachImpl(allocator, args);
    // If no error, the attach was successful
}

// Set Ready tests - ported from Rust set_ready tests
test "SetReady: marks activation as ready" {
    const allocator = testing.allocator;
    var temp_dir = try TempDir.init(allocator);
    defer temp_dir.deinit();
    
    const flox_env = "/path/to/floxenv";
    const store_path = "/store/path";
    const pid = 5678;
    
    // Create activation
    const activation_id = try createTestActivations(allocator, temp_dir.path, flox_env, store_path, pid);
    defer allocator.free(activation_id);
    
    const args = cli.SetReadyArgs{
        .flox_env = flox_env,
        .id = activation_id,
        .runtime_dir = temp_dir.path,
    };
    
    try activations.setReadyImpl(allocator, args);
    // If no error, the set ready was successful
}

// Shell generation tests - ported from Rust shell tests
test "Shell: parseShell valid shells" {
    try testing.expect(shell_gen.parseShell("bash") == .Bash);
    try testing.expect(shell_gen.parseShell("zsh") == .Zsh);
    try testing.expect(shell_gen.parseShell("fish") == .Fish);
    try testing.expect(shell_gen.parseShell("tcsh") == .Tcsh);
}

test "Shell: parseShell invalid shell returns null" {
    try testing.expect(shell_gen.parseShell("invalid") == null);
    try testing.expect(shell_gen.parseShell("") == null);
}

// Set env dirs tests - ported from Rust set_env_dirs tests
test "SetEnvDirs: skips adding duplicate flox_env" {
    const allocator = testing.allocator;
    const flox_env = "/foo/bar";
    const env_dirs = "/foo/bar:/baz:/qux";
    
    const result = try shell_gen.setEnvDirs(allocator, flox_env, env_dirs, .Bash);
    defer allocator.free(result);
    
    // Should only contain flox_env once
    const occurrences = std.mem.count(u8, result, "/foo/bar");
    try testing.expect(occurrences == 1);
    try testing.expect(std.mem.indexOf(u8, result, "/baz") != null);
    try testing.expect(std.mem.indexOf(u8, result, "/qux") != null);
}

test "SetEnvDirs: prepends to existing dirs" {
    const allocator = testing.allocator;
    const flox_env = "/foo";
    const env_dirs = "/bar:/baz";
    
    const result = try shell_gen.setEnvDirs(allocator, flox_env, env_dirs, .Bash);
    defer allocator.free(result);
    
    try testing.expect(std.mem.indexOf(u8, result, "FLOX_ENV_DIRS=\"/foo:/bar:/baz\"") != null);
}

test "SetEnvDirs: handles empty env_dirs" {
    const allocator = testing.allocator;
    const flox_env = "/foo";
    const env_dirs = "";
    
    const result = try shell_gen.setEnvDirs(allocator, flox_env, env_dirs, .Bash);
    defer allocator.free(result);
    
    try testing.expect(std.mem.indexOf(u8, result, "FLOX_ENV_DIRS=\"/foo\"") != null);
}

test "SetEnvDirs: lines have trailing semicolons" {
    const allocator = testing.allocator;
    const shells = [_]shell_gen.Shell{ .Bash, .Zsh, .Fish, .Tcsh };
    const flox_env = "/foo/bar";
    const env_dirs = "/env1:/env2";
    
    for (shells) |shell| {
        const result = try shell_gen.setEnvDirs(allocator, flox_env, env_dirs, shell);
        defer allocator.free(result);
        try testing.expect(std.mem.endsWith(u8, std.mem.trim(u8, result, "\n"), ";"));
    }
}

// Prepend and dedup tests - ported from Rust prepend_and_dedup tests  
test "PrependAndDedup: handles empty pathlike var" {
    const allocator = testing.allocator;
    const flox_env_dirs = "foo:bar";
    const suffixes = [_][]const u8{"bin"};
    
    const result = try shell_gen.prependAndDedup(allocator, flox_env_dirs, &suffixes, "", false);
    defer allocator.free(result);
    
    try testing.expect(std.mem.eql(u8, result, "foo/bin:bar/bin"));
}

test "PrependAndDedup: handles empty suffix" {
    const allocator = testing.allocator;
    const flox_env_dirs = "foo:bar";
    
    const result = try shell_gen.prependAndDedup(allocator, flox_env_dirs, null, "", false);
    defer allocator.free(result);
    
    try testing.expect(std.mem.eql(u8, result, "foo:bar"));
}

// Fix fpath tests - ported from Rust fix_fpath tests
test "FixFpath: makes space separated array" {
    const allocator = testing.allocator;
    const flox_env_dirs = "foo:bar";
    const fpath = "dir1:dir2";
    
    const result = try shell_gen.fixFpath(allocator, flox_env_dirs, fpath);
    defer allocator.free(result);
    
    const expected = "fpath=(\"foo/share/zsh/site-functions\" \"foo/share/zsh/vendor-completions\" \"bar/share/zsh/site-functions\" \"bar/share/zsh/vendor-completions\" \"dir1\" \"dir2\" )";
    try testing.expect(std.mem.eql(u8, result, expected));
}

// Fix paths tests - ported from Rust fix_paths tests
test "FixPaths: appends suffixes" {
    const allocator = testing.allocator;
    const flox_env_dirs = "/flox_env";
    const path = "/path1:/path2";
    const manpath = "/man1:/man2";
    
    const result = try shell_gen.fixPaths(allocator, flox_env_dirs, path, manpath, .Bash);
    defer allocator.free(result);
    
    try testing.expect(std.mem.indexOf(u8, result, "/flox_env/bin") != null);
    try testing.expect(std.mem.indexOf(u8, result, "/flox_env/share/man") != null);
    try testing.expect(std.mem.indexOf(u8, result, "/path1") != null);
    try testing.expect(std.mem.indexOf(u8, result, "/path2") != null);
}

test "FixPaths: manpath without trailing colon gets trailing colon" {
    const allocator = testing.allocator;
    const flox_env_dirs = "/foo:/bar";
    const path = "";
    const manpath = "/baz:/qux";
    
    const result = try shell_gen.fixPaths(allocator, flox_env_dirs, path, manpath, .Bash);
    defer allocator.free(result);
    
    try testing.expect(std.mem.indexOf(u8, result, "/foo/share/man:/bar/share/man:/baz:/qux:") != null);
}

test "FixPaths: manpath with trailing colon doesn't get new trailing colon" {
    const allocator = testing.allocator;
    const flox_env_dirs = "/foo:/bar";
    const path = "";
    const manpath = "/baz:/qux:";
    
    const result = try shell_gen.fixPaths(allocator, flox_env_dirs, path, manpath, .Bash);
    defer allocator.free(result);
    
    try testing.expect(std.mem.indexOf(u8, result, "/foo/share/man:/bar/share/man:/baz:/qux:") != null);
}

// Profile scripts tests - ported from Rust profile_scripts tests
test "ProfileScripts: bash all exist correct order" {
    const allocator = testing.allocator;
    const flox_env_dirs = "newer:older";
    const sourced = "";
    
    const result = try shell_gen.profileScripts(allocator, flox_env_dirs, sourced, .Bash);
    defer allocator.free(result);
    
    // Should source older first, then newer
    const older_pos = std.mem.indexOf(u8, result, "older/activate.d/profile-common") orelse return error.TestFailed;
    const newer_pos = std.mem.indexOf(u8, result, "newer/activate.d/profile-common") orelse return error.TestFailed;
    try testing.expect(older_pos < newer_pos);
}

test "ProfileScripts: one already sourced script is skipped" {
    const allocator = testing.allocator;
    const flox_env_dirs = "newer:older";
    const sourced = "older";
    
    const result = try shell_gen.profileScripts(allocator, flox_env_dirs, sourced, .Bash);
    defer allocator.free(result);
    
    // Should only source newer, not older
    try testing.expect(std.mem.indexOf(u8, result, "newer/activate.d/profile-common") != null);
    try testing.expect(std.mem.indexOf(u8, result, "older/activate.d/profile-common") == null);
}

test "ProfileScripts: all already sourced scripts are skipped" {
    const allocator = testing.allocator;
    const flox_env_dirs = "newer:older";
    const sourced = "newer:older";
    
    const result = try shell_gen.profileScripts(allocator, flox_env_dirs, sourced, .Bash);
    defer allocator.free(result);
    
    // Should not source any scripts, just update the variable
    try testing.expect(std.mem.indexOf(u8, result, "activate.d/profile-common") == null);
    try testing.expect(std.mem.indexOf(u8, result, "_FLOX_SOURCED_PROFILE_SCRIPTS") != null);
}