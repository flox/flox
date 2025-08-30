const std = @import("std");
const testing = std.testing;
const Allocator = std.mem.Allocator;

const cli = @import("cli.zig");
const activations = @import("activations.zig");
const shell_gen = @import("shell_gen.zig");

// Memory safety test allocator that tracks all allocations
const TrackingAllocator = struct {
    backing_allocator: Allocator,
    allocations: std.HashMap(usize, usize),
    mutex: std.Thread.Mutex,
    total_allocated: usize,
    total_freed: usize,
    
    const Self = @This();
    
    pub fn init(backing_allocator: Allocator) Self {
        return Self{
            .backing_allocator = backing_allocator,
            .allocations = std.HashMap(usize, usize).init(backing_allocator),
            .mutex = std.Thread.Mutex{},
            .total_allocated = 0,
            .total_freed = 0,
        };
    }
    
    pub fn deinit(self: *Self) void {
        self.allocations.deinit();
    }
    
    pub fn allocator(self: *Self) Allocator {
        return .{
            .ptr = self,
            .vtable = &.{
                .alloc = alloc,
                .resize = resize,
                .free = free,
            },
        };
    }
    
    fn alloc(ctx: *anyopaque, len: usize, log2_align: u8, return_address: usize) ?[*]u8 {
        const self: *Self = @ptrCast(@alignCast(ctx));
        const result = self.backing_allocator.vtable.alloc(self.backing_allocator.ptr, len, log2_align, return_address);
        
        if (result) |ptr| {
            self.mutex.lock();
            defer self.mutex.unlock();
            
            self.allocations.put(@intFromPtr(ptr), len) catch {};
            self.total_allocated += len;
        }
        
        return result;
    }
    
    fn resize(ctx: *anyopaque, buf: []u8, log2_align: u8, new_len: usize, return_address: usize) bool {
        const self: *Self = @ptrCast(@alignCast(ctx));
        
        if (self.backing_allocator.vtable.resize(self.backing_allocator.ptr, buf, log2_align, new_len, return_address)) {
            self.mutex.lock();
            defer self.mutex.unlock();
            
            const ptr_int = @intFromPtr(buf.ptr);
            if (self.allocations.get(ptr_int)) |old_len| {
                _ = self.allocations.remove(ptr_int);
                self.allocations.put(ptr_int, new_len) catch {};
                
                if (new_len > old_len) {
                    self.total_allocated += new_len - old_len;
                } else {
                    self.total_freed += old_len - new_len;
                }
            }
            return true;
        }
        return false;
    }
    
    fn free(ctx: *anyopaque, buf: []u8, log2_align: u8, return_address: usize) void {
        const self: *Self = @ptrCast(@alignCast(ctx));
        
        self.mutex.lock();
        const ptr_int = @intFromPtr(buf.ptr);
        if (self.allocations.get(ptr_int)) |len| {
            _ = self.allocations.remove(ptr_int);
            self.total_freed += len;
        }
        self.mutex.unlock();
        
        self.backing_allocator.vtable.free(self.backing_allocator.ptr, buf, log2_align, return_address);
    }
    
    pub fn checkLeaks(self: *const Self) !void {
        self.mutex.lock();
        defer self.mutex.unlock();
        
        if (self.allocations.count() > 0) {
            std.debug.print("Memory leak detected! {} allocations not freed\n", .{self.allocations.count()});
            var iter = self.allocations.iterator();
            while (iter.next()) |entry| {
                std.debug.print("  Leaked: ptr=0x{x}, size={}\n", .{ entry.key_ptr.*, entry.value_ptr.* });
            }
            return error.MemoryLeak;
        }
        
        std.debug.print("Memory check passed: allocated={}, freed={}\n", .{ self.total_allocated, self.total_freed });
    }
};

// Memory safety tests
test "Memory: no leaks in CLI argument parsing" {
    var tracking_allocator = TrackingAllocator.init(testing.allocator);
    defer tracking_allocator.deinit();
    const allocator = tracking_allocator.allocator();
    
    const args = [_][]const u8{ 
        "flox-activations", 
        "start-or-attach", 
        "--pid", "1234", 
        "--flox-env", "/path", 
        "--store-path", "/store", 
        "--runtime-dir", "/runtime" 
    };
    
    const result = try cli.parseArgs(allocator, &args);
    defer result.deinit(allocator);
    
    try tracking_allocator.checkLeaks();
}

test "Memory: no leaks in activation creation and cleanup" {
    var tracking_allocator = TrackingAllocator.init(testing.allocator);
    defer tracking_allocator.deinit();
    const allocator = tracking_allocator.allocator();
    
    var activations_data = activations.Activations.init(allocator);
    defer activations_data.deinit(allocator);
    
    // Create and remove multiple activations
    const activation1 = try activations_data.createActivation(allocator, "/store/path1", 1234);
    const activation2 = try activations_data.createActivation(allocator, "/store/path2", 5678);
    
    const id1 = try allocator.dupe(u8, activation1.id);
    defer allocator.free(id1);
    const id2 = try allocator.dupe(u8, activation2.id);
    defer allocator.free(id2);
    
    try activations_data.removeActivation(allocator, id1);
    try activations_data.removeActivation(allocator, id2);
    
    try tracking_allocator.checkLeaks();
}

test "Memory: no leaks in shell generation functions" {
    var tracking_allocator = TrackingAllocator.init(testing.allocator);
    defer tracking_allocator.deinit();
    const allocator = tracking_allocator.allocator();
    
    // Test all shell generation functions
    {
        const result = try shell_gen.setEnvDirs(allocator, "/foo", "/bar:/baz", .Bash);
        defer allocator.free(result);
    }
    
    {
        const result = try shell_gen.fixPaths(allocator, "/foo:/bar", "/path1:/path2", "/man1:/man2", .Bash);
        defer allocator.free(result);
    }
    
    {
        const result = try shell_gen.profileScripts(allocator, "/env1:/env2", "", .Bash);
        defer allocator.free(result);
    }
    
    {
        const suffixes = [_][]const u8{"bin"};
        const result = try shell_gen.prependAndDedup(allocator, "/env1:/env2", &suffixes, "/existing", false);
        defer allocator.free(result);
    }
    
    {
        const result = try shell_gen.fixFpath(allocator, "/env1:/env2", "/existing/fpath");
        defer allocator.free(result);
    }
    
    try tracking_allocator.checkLeaks();
}

test "Memory: no invalid memory access in string operations" {
    const allocator = testing.allocator;
    
    // Test with various edge cases that might cause invalid memory access
    const test_cases = [_]struct {
        env_dirs: []const u8,
        path: []const u8,
        expected_success: bool,
    }{
        .{ .env_dirs = "", .path = "", .expected_success = true },
        .{ .env_dirs = "single", .path = "single", .expected_success = true },
        .{ .env_dirs = "a:b:c:d:e", .path = "1:2:3:4:5", .expected_success = true },
        .{ .env_dirs = "empty", .path = "empty", .expected_success = true },
        .{ .env_dirs = ":::", .path = ":::", .expected_success = true },
    };
    
    for (test_cases) |case| {
        const result = shell_gen.setEnvDirs(allocator, "/test", case.env_dirs, .Bash) catch |err| {
            if (case.expected_success) {
                return err;
            }
            continue;
        };
        defer allocator.free(result);
        
        if (!case.expected_success) {
            return error.ExpectedFailure;
        }
    }
}

test "Memory: stress test with many allocations" {
    var tracking_allocator = TrackingAllocator.init(testing.allocator);
    defer tracking_allocator.deinit();
    const allocator = tracking_allocator.allocator();
    
    var activations_data = activations.Activations.init(allocator);
    defer activations_data.deinit(allocator);
    
    // Create many activations and clean them up
    const num_activations = 100;
    var activation_ids = try allocator.alloc([]u8, num_activations);
    defer {
        for (activation_ids) |id| {
            allocator.free(id);
        }
        allocator.free(activation_ids);
    }
    
    // Create activations
    for (0..num_activations) |i| {
        const store_path = try std.fmt.allocPrint(allocator, "/store/path/{}", .{i});
        defer allocator.free(store_path);
        
        const activation = try activations_data.createActivation(allocator, store_path, @intCast(i32, i + 1000));
        activation_ids[i] = try allocator.dupe(u8, activation.id);
    }
    
    // Remove all activations
    for (activation_ids) |id| {
        try activations_data.removeActivation(allocator, id);
    }
    
    try testing.expect(activations_data.isEmpty());
    try tracking_allocator.checkLeaks();
}

test "Memory: no buffer overflow in string concatenation" {
    const allocator = testing.allocator;
    
    // Test with very long strings to check for buffer overflows
    var long_path = try allocator.alloc(u8, 4096);
    defer allocator.free(long_path);
    std.mem.set(u8, long_path, 'a');
    long_path[long_path.len - 1] = 0; // null terminate
    
    const long_path_str = long_path[0..long_path.len-1];
    
    // This should either succeed or fail gracefully, not cause buffer overflow
    const result = shell_gen.setEnvDirs(allocator, long_path_str, long_path_str, .Bash) catch |err| {
        // Expected to potentially fail with long paths
        try testing.expect(err == error.OutOfMemory or err == error.InvalidArgument);
        return;
    };
    defer allocator.free(result);
    
    // If it succeeds, the result should be valid
    try testing.expect(result.len > 0);
}

test "Memory: double free protection" {
    const allocator = testing.allocator;
    
    var activations_data = activations.Activations.init(allocator);
    defer activations_data.deinit(allocator);
    
    const activation = try activations_data.createActivation(allocator, "/store/path", 1234);
    const id = try allocator.dupe(u8, activation.id);
    defer allocator.free(id);
    
    // Remove the activation
    try activations_data.removeActivation(allocator, id);
    
    // Trying to remove again should not cause double free
    // (should either be no-op or return error gracefully)
    activations_data.removeActivation(allocator, id) catch |err| {
        // Expected to potentially fail when removing non-existent activation
        try testing.expect(err == error.ActivationNotFound or err == error.InvalidArgument);
    };
}

test "Memory: use after free protection" {
    const allocator = testing.allocator;
    
    var activations_data = activations.Activations.init(allocator);
    defer activations_data.deinit(allocator);
    
    const activation = try activations_data.createActivation(allocator, "/store/path", 1234);
    const id = try allocator.dupe(u8, activation.id);
    defer allocator.free(id);
    
    // Remove the activation
    try activations_data.removeActivation(allocator, id);
    
    // Trying to access the removed activation should return null or error safely
    const found = activations_data.findActivationById(id);
    try testing.expect(found == null);
}

// Fuzzing-style tests to catch edge cases
test "Memory: fuzz test string splitting" {
    const allocator = testing.allocator;
    
    const fuzz_inputs = [_][]const u8{
        "",
        ":",
        "::",
        ":::",
        "a:",
        ":a",
        "a:b",
        "a::b",
        "a:::b",
        "very_long_path_name_that_might_cause_issues_if_not_handled_properly",
        "path/with/many/slashes/and/components",
    };
    
    for (fuzz_inputs) |input| {
        const result = shell_gen.setEnvDirs(allocator, "/test", input, .Bash) catch |err| {
            // Should fail gracefully, not crash
            try testing.expect(err == error.OutOfMemory or err == error.InvalidArgument);
            continue;
        };
        defer allocator.free(result);
        
        // Result should be valid
        try testing.expect(result.len > 0);
    }
}

test "Memory: concurrent access safety" {
    const allocator = testing.allocator;
    
    var activations_data = activations.Activations.init(allocator);
    defer activations_data.deinit(allocator);
    
    // Simulate concurrent access by creating and modifying activations rapidly
    for (0..50) |i| {
        const store_path = try std.fmt.allocPrint(allocator, "/store/{}", .{i});
        defer allocator.free(store_path);
        
        const activation = try activations_data.createActivation(allocator, store_path, @intCast(i32, i + 1000));
        
        // Immediately modify the activation
        try activations.addPidToActivation(allocator, activation, @intCast(i32, i + 2000), null);
        
        // Remove some activations to simulate cleanup
        if (i % 3 == 0) {
            const id = try allocator.dupe(u8, activation.id);
            defer allocator.free(id);
            try activations_data.removeActivation(allocator, id);
        }
    }
}

// Test boundary conditions that might cause memory issues
test "Memory: boundary conditions" {
    const allocator = testing.allocator;
    
    // Test with zero-length arguments
    {
        const result = shell_gen.setEnvDirs(allocator, "", "", .Bash) catch |err| {
            try testing.expect(err == error.InvalidArgument);
            return;
        };
        defer allocator.free(result);
    }
    
    // Test with maximum reasonable path lengths
    {
        const max_path = try allocator.alloc(u8, 4095); // PATH_MAX - 1
        defer allocator.free(max_path);
        std.mem.set(u8, max_path, 'x');
        
        const result = shell_gen.setEnvDirs(allocator, max_path, "", .Bash) catch |err| {
            try testing.expect(err == error.OutOfMemory or err == error.InvalidArgument);
            return;
        };
        defer allocator.free(result);
    }
    
    // Test with many path components
    {
        var many_paths = std.ArrayList(u8).init(allocator);
        defer many_paths.deinit();
        
        for (0..1000) |i| {
            if (i > 0) try many_paths.append(':');
            try many_paths.writer().print("/path{}", .{i});
        }
        
        const result = shell_gen.setEnvDirs(allocator, "/test", many_paths.items, .Bash) catch |err| {
            try testing.expect(err == error.OutOfMemory);
            return;
        };
        defer allocator.free(result);
    }
}

test "Memory: activation lifecycle without leaks" {
    var tracking_allocator = TrackingAllocator.init(testing.allocator);
    defer tracking_allocator.deinit();
    const allocator = tracking_allocator.allocator();
    
    // Simulate complete activation lifecycle
    var activations_data = activations.Activations.init(allocator);
    defer activations_data.deinit(allocator);
    
    // 1. Create activation
    const activation = try activations_data.createActivation(allocator, "/store/path", 1234);
    const id = try allocator.dupe(u8, activation.id);
    defer allocator.free(id);
    
    // 2. Attach more PIDs
    try activations.addPidToActivation(allocator, activation, 5678, null);
    try activations.addPidToActivation(allocator, activation, 9012, std.time.milliTimestamp() + 1000);
    
    // 3. Remove some PIDs
    try activations.removePidFromActivation(allocator, activation, 5678);
    
    // 4. Set ready
    activation.ready = true;
    
    // 5. Remove activation completely
    try activations_data.removeActivation(allocator, id);
    
    try tracking_allocator.checkLeaks();
}