const std = @import("std");
const Allocator = std.mem.Allocator;
const ArrayList = std.ArrayList;

const cli = @import("cli.zig");
const json = @import("json.zig");
const Error = @import("main.zig").Error;

const LATEST_VERSION: u8 = 1;

pub const AttachedPid = struct {
    pid: i32,
    expiration: ?i64, // Unix timestamp in milliseconds
};

pub const Activation = struct {
    id: []const u8,
    store_path: []const u8,
    ready: bool,
    attached_pids: []AttachedPid,
    
    pub fn deinit(self: *Activation, allocator: Allocator) void {
        allocator.free(self.id);
        allocator.free(self.store_path);
        for (self.attached_pids) |_| {
            // AttachedPid doesn't own any memory
        }
        allocator.free(self.attached_pids);
    }
    
    pub fn isStartupProcessRunning(self: *const Activation) bool {
        // Check if any of the attached PIDs are still running
        for (self.attached_pids) |attached_pid| {
            if (isPidRunning(attached_pid.pid)) {
                return true;
            }
        }
        return false;
    }
};

pub const Activations = struct {
    version: u8,
    activations: []Activation,
    
    pub fn init(allocator: Allocator) Activations {
        return Activations{
            .version = LATEST_VERSION,
            .activations = allocator.alloc(Activation, 0) catch unreachable,
        };
    }
    
    pub fn deinit(self: *Activations, allocator: Allocator) void {
        for (self.activations) |*activation| {
            activation.deinit(allocator);
        }
        allocator.free(self.activations);
    }
    
    pub fn findActivationByStorePath(self: *const Activations, store_path: []const u8) ?*const Activation {
        for (self.activations) |*activation| {
            if (std.mem.eql(u8, activation.store_path, store_path)) {
                return activation;
            }
        }
        return null;
    }
    
    pub fn findActivationByStorePathMut(self: *Activations, store_path: []const u8) ?*Activation {
        for (self.activations) |*activation| {
            if (std.mem.eql(u8, activation.store_path, store_path)) {
                return activation;
            }
        }
        return null;
    }
    
    pub fn findActivationById(self: *const Activations, id: []const u8) ?*const Activation {
        for (self.activations) |*activation| {
            if (std.mem.eql(u8, activation.id, id)) {
                return activation;
            }
        }
        return null;
    }
    
    pub fn findActivationByIdMut(self: *Activations, id: []const u8) ?*Activation {
        for (self.activations) |*activation| {
            if (std.mem.eql(u8, activation.id, id)) {
                return activation;
            }
        }
        return null;
    }
    
    pub fn createActivation(self: *Activations, allocator: Allocator, store_path: []const u8, pid: i32) !*Activation {
        // Generate unique ID
        const id = try generateActivationId(allocator);
        
        // Create new activation
        const activation = Activation{
            .id = id,
            .store_path = try allocator.dupe(u8, store_path),
            .ready = false,
            .attached_pids = try allocator.alloc(AttachedPid, 1),
        };
        
        // Set the initial attached PID
        activation.attached_pids[0] = AttachedPid{ .pid = pid, .expiration = null };
        
        // Add to activations list
        const new_activations = try allocator.realloc(self.activations, self.activations.len + 1);
        new_activations[new_activations.len - 1] = activation;
        self.activations = new_activations;
        
        return &self.activations[self.activations.len - 1];
    }
    
    pub fn removeActivation(self: *Activations, allocator: Allocator, id: []const u8) !void {
        var found_idx: ?usize = null;
        for (self.activations, 0..) |*activation, i| {
            if (std.mem.eql(u8, activation.id, id)) {
                found_idx = i;
                break;
            }
        }
        
        if (found_idx) |idx| {
            // First clean up the activation we're removing
            self.activations[idx].deinit(allocator);
            
            // Then shift remaining elements
            for (idx..self.activations.len - 1) |i| {
                self.activations[i] = self.activations[i + 1];
            }
            self.activations = try allocator.realloc(self.activations, self.activations.len - 1);
        }
    }
    
    pub fn isEmpty(self: *const Activations) bool {
        return self.activations.len == 0;
    }
};

pub const FileLock = struct {
    file: std.fs.File,
    
    pub fn init(path: []const u8) !FileLock {
        const file = try std.fs.cwd().createFile(path, .{ .read = true });
        // TODO: Implement file locking using fcntl or similar
        return FileLock{ .file = file };
    }
    
    pub fn deinit(self: *FileLock) void {
        self.file.close();
    }
};

pub const StartOrAttachResult = struct {
    attaching: bool,
    state_dir: []const u8,
    activation_id: []const u8,
    
    pub fn deinit(self: *const StartOrAttachResult, allocator: Allocator) void {
        allocator.free(self.state_dir);
        allocator.free(self.activation_id);
    }
};

// Implementation functions
pub fn startOrAttachImpl(allocator: Allocator, args: cli.StartOrAttachArgs) !StartOrAttachResult {
    const activations_path = try getActivationsJsonPath(allocator, args.runtime_dir, args.flox_env);
    defer allocator.free(activations_path);
    
    var activations_data = readActivationsJson(allocator, activations_path) catch |err| switch (err) {
        Error.FileNotFound => Activations.init(allocator),
        else => return err,
    };
    defer activations_data.deinit(allocator);
    
    // Check if activation already exists for this store path
    if (activations_data.findActivationByStorePath(args.store_path)) |existing_activation| {
        // Attach to existing activation
        try attachToActivation(allocator, &activations_data, existing_activation.id, args.pid);
        try writeActivationsJson(allocator, &activations_data, activations_path);
        
        const state_dir = try getActivationStateDirPath(allocator, args.runtime_dir, args.flox_env, existing_activation.id);
        return StartOrAttachResult{
            .attaching = true,
            .state_dir = state_dir,
            .activation_id = try allocator.dupe(u8, existing_activation.id),
        };
    } else {
        // Start new activation
        const new_activation = try activations_data.createActivation(allocator, args.store_path, args.pid);
        
        // Create state directory
        const state_dir = try getActivationStateDirPath(allocator, args.runtime_dir, args.flox_env, new_activation.id);
        defer allocator.free(state_dir);
        try std.fs.cwd().makePath(state_dir);
        
        try writeActivationsJson(allocator, &activations_data, activations_path);
        
        return StartOrAttachResult{
            .attaching = false,
            .state_dir = try allocator.dupe(u8, state_dir),
            .activation_id = try allocator.dupe(u8, new_activation.id),
        };
    }
}

pub fn setReadyImpl(allocator: Allocator, args: cli.SetReadyArgs) !void {
    const activations_path = try getActivationsJsonPath(allocator, args.runtime_dir, args.flox_env);
    defer allocator.free(activations_path);
    
    var activations_data = try readActivationsJson(allocator, activations_path);
    defer activations_data.deinit(allocator);
    
    const activation = activations_data.findActivationByIdMut(args.id) orelse return Error.ActivationError;
    activation.ready = true;
    
    try writeActivationsJson(allocator, &activations_data, activations_path);
}

pub fn attachImpl(allocator: Allocator, args: cli.AttachArgs) !void {
    const activations_path = try getActivationsJsonPath(allocator, args.runtime_dir, args.flox_env);
    defer allocator.free(activations_path);
    
    var activations_data = try readActivationsJson(allocator, activations_path);
    defer activations_data.deinit(allocator);
    
    const activation = activations_data.findActivationByIdMut(args.id) orelse return Error.ActivationError;
    
    // Handle remove_pid if specified
    if (args.remove_pid) |remove_pid| {
        try removePidFromActivation(allocator, activation, remove_pid);
    }
    
    // Add the new PID
    const expiration = if (args.timeout_ms) |timeout| 
        std.time.milliTimestamp() + @as(i64, timeout)
    else 
        null;
        
    try addPidToActivation(allocator, activation, args.pid, expiration);
    
    try writeActivationsJson(allocator, &activations_data, activations_path);
}

// Helper functions
pub fn getActivationsJsonPath(allocator: Allocator, runtime_dir: []const u8, flox_env: []const u8) ![]u8 {
    const env_hash = try pathHash(allocator, flox_env);
    defer allocator.free(env_hash);
    return try std.fmt.allocPrint(allocator, "{s}/{s}/activations.json", .{ runtime_dir, env_hash });
}

fn getActivationStateDirPath(allocator: Allocator, runtime_dir: []const u8, flox_env: []const u8, activation_id: []const u8) ![]u8 {
    const env_hash = try pathHash(allocator, flox_env);
    defer allocator.free(env_hash);
    return try std.fmt.allocPrint(allocator, "{s}/{s}/{s}", .{ runtime_dir, env_hash, activation_id });
}

fn pathHash(allocator: Allocator, path: []const u8) ![]u8 {
    // Simple hash implementation - in production, use a proper hash function
    var hash = std.hash.Wyhash.init(0);
    hash.update(path);
    const hash_value = hash.final();
    return try std.fmt.allocPrint(allocator, "{x}", .{hash_value});
}

fn generateActivationId(allocator: Allocator) ![]u8 {
    // Simple ID generation - in production, use UUID or similar
    const timestamp = std.time.milliTimestamp();
    return try std.fmt.allocPrint(allocator, "{}", .{timestamp});
}

pub fn readActivationsJson(allocator: Allocator, path: []const u8) !Activations {
    const file = std.fs.cwd().openFile(path, .{}) catch |err| switch (err) {
        error.FileNotFound => return Error.FileNotFound,
        else => return err,
    };
    defer file.close();
    
    const contents = try file.readToEndAlloc(allocator, 1024 * 1024);
    defer allocator.free(contents);
    
    return try json.deserializeActivations(allocator, contents);
}

pub fn writeActivationsJson(allocator: Allocator, data: *const Activations, path: []const u8) !void {
    // Create parent directory if it doesn't exist
    const dirname = std.fs.path.dirname(path) orelse return Error.FileNotFound;
    try std.fs.cwd().makePath(dirname);
    
    const file = try std.fs.cwd().createFile(path, .{});
    defer file.close();
    
    const json_content = try json.serializeActivations(allocator, data);
    defer allocator.free(json_content);
    try file.writeAll(json_content);
}

fn attachToActivation(allocator: Allocator, activations: *Activations, activation_id: []const u8, pid: i32) !void {
    const activation = activations.findActivationByIdMut(activation_id) orelse return Error.ActivationError;
    try addPidToActivation(allocator, activation, pid, null);
}

pub fn addPidToActivation(allocator: Allocator, activation: *Activation, pid: i32, expiration: ?i64) !void {
    const new_pids = try allocator.realloc(activation.attached_pids, activation.attached_pids.len + 1);
    new_pids[new_pids.len - 1] = AttachedPid{ .pid = pid, .expiration = expiration };
    activation.attached_pids = new_pids;
}

pub fn removePidFromActivation(allocator: Allocator, activation: *Activation, pid: i32) !void {
    var found_idx: ?usize = null;
    for (activation.attached_pids, 0..) |attached_pid, i| {
        if (attached_pid.pid == pid) {
            found_idx = i;
            break;
        }
    }
    
    if (found_idx) |idx| {
        // Shift remaining elements
        for (idx..activation.attached_pids.len - 1) |i| {
            activation.attached_pids[i] = activation.attached_pids[i + 1];
        }
        activation.attached_pids = try allocator.realloc(activation.attached_pids, activation.attached_pids.len - 1);
    }
}

fn isPidRunning(pid: i32) bool {
    // Check if PID is running by trying to send signal 0
    const result = std.os.kill(pid, 0);
    return switch (std.os.errno(result)) {
        .SUCCESS => true,
        .SRCH => false, // No such process
        .PERM => true,  // Process exists but no permission
        else => false,
    };
}

// Tests
test "Activations init and basic operations" {
    const allocator = std.testing.allocator;
    
    var activations = Activations.init(allocator);
    defer activations.deinit(allocator);
    
    try std.testing.expect(activations.isEmpty());
    try std.testing.expect(activations.version == LATEST_VERSION);
}

test "createActivation and findActivation" {
    const allocator = std.testing.allocator;
    
    var activations = Activations.init(allocator);
    defer activations.deinit(allocator);
    
    const activation = try activations.createActivation(allocator, "/store/path", 1234);
    try std.testing.expect(!activations.isEmpty());
    try std.testing.expect(std.mem.eql(u8, activation.store_path, "/store/path"));
    try std.testing.expect(activation.attached_pids.len == 1);
    try std.testing.expect(activation.attached_pids[0].pid == 1234);
    
    const found = activations.findActivationByStorePath("/store/path");
    try std.testing.expect(found != null);
    try std.testing.expect(std.mem.eql(u8, found.?.id, activation.id));
}

test "pathHash generates consistent hashes" {
    const allocator = std.testing.allocator;
    
    const hash1 = try pathHash(allocator, "/path/to/test");
    defer allocator.free(hash1);
    
    const hash2 = try pathHash(allocator, "/path/to/test");
    defer allocator.free(hash2);
    
    try std.testing.expect(std.mem.eql(u8, hash1, hash2));
    
    const hash3 = try pathHash(allocator, "/different/path");
    defer allocator.free(hash3);
    
    try std.testing.expect(!std.mem.eql(u8, hash1, hash3));
}