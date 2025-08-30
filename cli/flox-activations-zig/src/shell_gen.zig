const std = @import("std");
const Allocator = std.mem.Allocator;
const ArrayList = std.ArrayList;

const Error = @import("main.zig").Error;

pub const Shell = enum {
    Bash,
    Zsh,
    Fish,
    Tcsh,
    
    pub fn format(self: Shell, comptime fmt: []const u8, options: std.fmt.FormatOptions, writer: anytype) !void {
        _ = fmt;
        _ = options;
        switch (self) {
            .Bash => try writer.writeAll("bash"),
            .Zsh => try writer.writeAll("zsh"),
            .Fish => try writer.writeAll("fish"),
            .Tcsh => try writer.writeAll("tcsh"),
        }
    }
};

pub fn parseShell(shell_str: []const u8) ?Shell {
    if (std.mem.eql(u8, shell_str, "bash")) return .Bash;
    if (std.mem.eql(u8, shell_str, "zsh")) return .Zsh;
    if (std.mem.eql(u8, shell_str, "fish")) return .Fish;
    if (std.mem.eql(u8, shell_str, "tcsh")) return .Tcsh;
    return null;
}

pub fn fixPaths(allocator: Allocator, flox_env_dirs: []const u8, path: []const u8, manpath: []const u8, shell: Shell) ![]u8 {
    var result = ArrayList(u8).init(allocator);
    defer result.deinit();
    
    // Split flox_env_dirs by ':'
    var dir_iter = std.mem.splitAny(u8, flox_env_dirs, ":");
    var new_path_parts = ArrayList([]const u8).init(allocator);
    defer new_path_parts.deinit();
    var new_manpath_parts = ArrayList([]const u8).init(allocator);
    defer new_manpath_parts.deinit();
    
    // Add bin and share/man paths from each flox env dir
    while (dir_iter.next()) |dir| {
        if (dir.len == 0) continue;
        const bin_path = try std.fmt.allocPrint(allocator, "{s}/bin", .{dir});
        defer allocator.free(bin_path);
        try new_path_parts.append(try allocator.dupe(u8, bin_path));
        
        const man_path = try std.fmt.allocPrint(allocator, "{s}/share/man", .{dir});
        defer allocator.free(man_path);
        try new_manpath_parts.append(try allocator.dupe(u8, man_path));
    }
    
    // Add existing PATH entries
    if (path.len > 0 and !std.mem.eql(u8, path, "empty")) {
        var path_iter = std.mem.splitAny(u8, path, ":");
        while (path_iter.next()) |p| {
            if (p.len == 0) continue;
            try new_path_parts.append(try allocator.dupe(u8, p));
        }
    }
    
    // Add existing MANPATH entries
    if (manpath.len > 0 and !std.mem.eql(u8, manpath, "empty")) {
        var manpath_iter = std.mem.splitAny(u8, manpath, ":");
        while (manpath_iter.next()) |mp| {
            if (mp.len == 0) continue;
            try new_manpath_parts.append(try allocator.dupe(u8, mp));
        }
    }
    
    // Deduplicate paths
    try deduplicateSlice(allocator, new_path_parts.items);
    try deduplicateSlice(allocator, new_manpath_parts.items);
    
    // Generate shell-specific output
    const writer = result.writer();
    switch (shell) {
        .Bash, .Zsh => {
            const joined_path = try std.mem.join(allocator, ":", new_path_parts.items);
            defer allocator.free(joined_path);
            try writer.print("PATH=\"{s}\";\n", .{joined_path});
            
            const joined_manpath = try std.mem.join(allocator, ":", new_manpath_parts.items);
            defer allocator.free(joined_manpath);
            
            // Add trailing colon if not present
            const needs_colon = !std.mem.endsWith(u8, joined_manpath, ":");
            if (needs_colon) {
                try writer.print("MANPATH=\"{s}:\";\n", .{joined_manpath});
            } else {
                try writer.print("MANPATH=\"{s}\";\n", .{joined_manpath});
            }
        },
        .Fish => {
            const joined_path = try std.mem.join(allocator, ":", new_path_parts.items);
            defer allocator.free(joined_path);
            try writer.print("set -gx PATH \"{s}\";\n", .{joined_path});
            
            const joined_manpath = try std.mem.join(allocator, ":", new_manpath_parts.items);
            defer allocator.free(joined_manpath);
            
            const needs_colon = !std.mem.endsWith(u8, joined_manpath, ":");
            if (needs_colon) {
                try writer.print("set -gx MANPATH \"{s}:\";\n", .{joined_manpath});
            } else {
                try writer.print("set -gx MANPATH \"{s}\";\n", .{joined_manpath});
            }
        },
        .Tcsh => {
            const joined_path = try std.mem.join(allocator, ":", new_path_parts.items);
            defer allocator.free(joined_path);
            try writer.print("setenv PATH \"{s}\";\n", .{joined_path});
            
            const joined_manpath = try std.mem.join(allocator, ":", new_manpath_parts.items);
            defer allocator.free(joined_manpath);
            
            const needs_colon = !std.mem.endsWith(u8, joined_manpath, ":");
            if (needs_colon) {
                try writer.print("setenv MANPATH \"{s}:\";\n", .{joined_manpath});
            } else {
                try writer.print("setenv MANPATH \"{s}\";\n", .{joined_manpath});
            }
        },
    }
    
    // Clean up allocated path parts
    for (new_path_parts.items) |part| {
        allocator.free(part);
    }
    for (new_manpath_parts.items) |part| {
        allocator.free(part);
    }
    
    return try allocator.dupe(u8, result.items);
}

pub fn setEnvDirs(allocator: Allocator, flox_env: []const u8, env_dirs: []const u8, shell: Shell) ![]u8 {
    var result = ArrayList(u8).init(allocator);
    defer result.deinit();
    
    var dirs = ArrayList([]const u8).init(allocator);
    defer dirs.deinit();
    
    // Always prepend flox_env
    try dirs.append(flox_env);
    
    // Add existing env_dirs if not empty
    if (env_dirs.len > 0 and !std.mem.eql(u8, env_dirs, "empty")) {
        var iter = std.mem.splitAny(u8, env_dirs, ":");
        while (iter.next()) |dir| {
            if (dir.len == 0) continue;
            // Skip if already in list (dedup)
            var found = false;
            for (dirs.items) |existing| {
                if (std.mem.eql(u8, existing, dir)) {
                    found = true;
                    break;
                }
            }
            if (!found) {
                try dirs.append(dir);
            }
        }
    }
    
    const joined = try std.mem.join(allocator, ":", dirs.items);
    defer allocator.free(joined);
    
    const writer = result.writer();
    switch (shell) {
        .Bash, .Zsh => try writer.print("FLOX_ENV_DIRS=\"{s}\";\n", .{joined}),
        .Fish => try writer.print("set -gx FLOX_ENV_DIRS \"{s}\";\n", .{joined}),
        .Tcsh => try writer.print("setenv FLOX_ENV_DIRS \"{s}\";\n", .{joined}),
    }
    
    return try allocator.dupe(u8, result.items);
}

pub fn profileScripts(allocator: Allocator, flox_env_dirs: []const u8, sourced_profile_scripts: []const u8, shell: Shell) ![]u8 {
    var result = ArrayList(u8).init(allocator);
    defer result.deinit();
    const writer = result.writer();
    
    // Parse directories in reverse order (older first)
    var dirs = ArrayList([]const u8).init(allocator);
    defer dirs.deinit();
    
    var iter = std.mem.splitAny(u8, flox_env_dirs, ":");
    while (iter.next()) |dir| {
        if (dir.len == 0) continue;
        try dirs.append(dir);
    }
    
    // Reverse to get older dirs first
    std.mem.reverse([]const u8, dirs.items);
    
    // Parse already sourced scripts
    var sourced = ArrayList([]const u8).init(allocator);
    defer sourced.deinit();
    
    if (sourced_profile_scripts.len > 0) {
        var sourced_iter = std.mem.splitAny(u8, sourced_profile_scripts, ":");
        while (sourced_iter.next()) |script| {
            if (script.len == 0) continue;
            try sourced.append(script);
        }
    }
    
    // Generate source commands for each profile script
    
    for (dirs.items) |dir| {
        // Skip if directory already sourced
        var already_sourced = false;
        for (sourced.items) |sourced_dir| {
            if (std.mem.eql(u8, dir, sourced_dir)) {
                already_sourced = true;
                break;
            }
        }
        if (already_sourced) continue;
        
        // Source common script first
        const common_path = try std.fmt.allocPrint(allocator, "{s}/activate.d/profile-common", .{dir});
        defer allocator.free(common_path);
        
        switch (shell) {
            .Bash, .Zsh => try writer.print("source '{s}';\n", .{common_path}),
            .Fish => try writer.print("source '{s}';\n", .{common_path}),
            .Tcsh => try writer.print("source '{s}';\n", .{common_path}),
        }
        
        // Source shell-specific script
        const shell_script = switch (shell) {
            .Bash => "profile-bash",
            .Zsh => "profile-zsh",
            .Fish => "profile-fish",
            .Tcsh => "profile-tcsh",
        };
        
        const shell_path = try std.fmt.allocPrint(allocator, "{s}/activate.d/{s}", .{ dir, shell_script });
        defer allocator.free(shell_path);
        
        switch (shell) {
            .Bash, .Zsh => try writer.print("source '{s}';\n", .{shell_path}),
            .Fish => try writer.print("source '{s}';\n", .{shell_path}),
            .Tcsh => try writer.print("source '{s}';\n", .{shell_path}),
        }
    }
    
    // Update the sourced profile scripts variable
    const new_sourced = if (sourced_profile_scripts.len > 0) 
        try std.fmt.allocPrint(allocator, "{s}:{s}", .{ sourced_profile_scripts, flox_env_dirs })
    else 
        try allocator.dupe(u8, flox_env_dirs);
    defer allocator.free(new_sourced);
    
    switch (shell) {
        .Bash, .Zsh => try writer.print("_FLOX_SOURCED_PROFILE_SCRIPTS='{s}';\n", .{new_sourced}),
        .Fish => try writer.print("set -gx _FLOX_SOURCED_PROFILE_SCRIPTS '{s}';\n", .{new_sourced}),
        .Tcsh => try writer.print("setenv _FLOX_SOURCED_PROFILE_SCRIPTS '{s}';\n", .{new_sourced}),
    }
    
    return try allocator.dupe(u8, result.items);
}

pub fn prependAndDedup(allocator: Allocator, flox_env_dirs: []const u8, suffixes: ?[]const []const u8, pathlike_var: []const u8, prune: bool) ![]u8 {
    _ = prune; // Not implemented in this basic version
    
    var result = ArrayList([]const u8).init(allocator);
    defer result.deinit();
    
    // Split flox_env_dirs
    var dir_iter = std.mem.splitAny(u8, flox_env_dirs, ":");
    while (dir_iter.next()) |dir| {
        if (dir.len == 0) continue;
        
        if (suffixes) |suffix_list| {
            for (suffix_list) |suffix| {
                const path = try std.fmt.allocPrint(allocator, "{s}/{s}", .{ dir, suffix });
                defer allocator.free(path);
                try result.append(try allocator.dupe(u8, path));
            }
        } else {
            try result.append(try allocator.dupe(u8, dir));
        }
    }
    
    // Add existing pathlike_var entries
    if (pathlike_var.len > 0 and !std.mem.eql(u8, pathlike_var, "empty")) {
        var path_iter = std.mem.splitAny(u8, pathlike_var, ":");
        while (path_iter.next()) |path| {
            if (path.len == 0) continue;
            try result.append(try allocator.dupe(u8, path));
        }
    }
    
    // Deduplicate
    try deduplicateSlice(allocator, result.items);
    
    const joined = try std.mem.join(allocator, ":", result.items);
    defer allocator.free(joined);
    
    // Clean up allocated parts
    for (result.items) |part| {
        allocator.free(part);
    }
    
    return try allocator.dupe(u8, joined);
}

pub fn fixFpath(allocator: Allocator, flox_env_dirs: []const u8, fpath: []const u8) ![]u8 {
    var result = ArrayList(u8).init(allocator);
    defer result.deinit();
    const writer = result.writer();
    
    try writer.writeAll("fpath=(");
    
    // Add flox env dirs with zsh completion paths
    var dir_iter = std.mem.splitAny(u8, flox_env_dirs, ":");
    while (dir_iter.next()) |dir| {
        if (dir.len == 0) continue;
        try writer.print("\"{s}/share/zsh/site-functions\" \"{s}/share/zsh/vendor-completions\" ", .{ dir, dir });
    }
    
    // Add existing fpath entries
    if (fpath.len > 0 and !std.mem.eql(u8, fpath, "empty")) {
        var fpath_iter = std.mem.splitAny(u8, fpath, ":");
        while (fpath_iter.next()) |path| {
            if (path.len == 0) continue;
            try writer.print("\"{s}\" ", .{path});
        }
    }
    
    try writer.writeAll(")");
    
    return try allocator.dupe(u8, result.items);
}

// Helper function to deduplicate a slice of strings in-place
fn deduplicateSlice(allocator: Allocator, items: [][]const u8) !void {
    _ = allocator;
    var write_idx: usize = 0;
    
    for (items) |item| {
        var is_duplicate = false;
        
        // Check if this item already exists in the portion we've processed
        for (items[0..write_idx]) |existing| {
            if (std.mem.eql(u8, item, existing)) {
                is_duplicate = true;
                break;
            }
        }
        
        if (!is_duplicate) {
            items[write_idx] = item;
            write_idx += 1;
        }
    }
    
    // The slice now has unique elements from 0..write_idx
    // Note: The caller should only use items[0..write_idx] after this call
}

// Tests
test "parseShell valid shells" {
    try std.testing.expect(parseShell("bash") == .Bash);
    try std.testing.expect(parseShell("zsh") == .Zsh);
    try std.testing.expect(parseShell("fish") == .Fish);
    try std.testing.expect(parseShell("tcsh") == .Tcsh);
}

test "parseShell invalid shell" {
    try std.testing.expect(parseShell("invalid") == null);
}

test "setEnvDirs basic functionality" {
    const allocator = std.testing.allocator;
    const result = try setEnvDirs(allocator, "/foo", "bar:baz", .Bash);
    defer allocator.free(result);
    
    try std.testing.expect(std.mem.indexOf(u8, result, "FLOX_ENV_DIRS=\"/foo:bar:baz\"") != null);
}