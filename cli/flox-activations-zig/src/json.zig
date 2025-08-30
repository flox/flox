const std = @import("std");
const Allocator = std.mem.Allocator;

const activations = @import("activations.zig");
const Error = @import("main.zig").Error;

// Simple JSON serialization/deserialization for activations
// In production, you'd want to use a more robust JSON library

pub fn serializeActivations(allocator: Allocator, data: *const activations.Activations) ![]u8 {
    // Create serializable structures
    const SerializableAttachedPid = struct {
        pid: i32,
        expiration: ?i64,
    };
    
    const SerializableActivation = struct {
        id: []const u8,
        store_path: []const u8,
        ready: bool,
        attached_pids: []SerializableAttachedPid,
    };
    
    const SerializableActivations = struct {
        version: u8,
        activations: []SerializableActivation,
    };
    
    // Convert activations to serializable format
    var serializable_activations = try allocator.alloc(SerializableActivation, data.activations.len);
    defer allocator.free(serializable_activations);
    
    for (data.activations, 0..) |activation, i| {
        var serializable_pids = try allocator.alloc(SerializableAttachedPid, activation.attached_pids.len);
        defer allocator.free(serializable_pids);
        
        for (activation.attached_pids, 0..) |pid, j| {
            serializable_pids[j] = SerializableAttachedPid{
                .pid = pid.pid,
                .expiration = pid.expiration,
            };
        }
        
        serializable_activations[i] = SerializableActivation{
            .id = activation.id,
            .store_path = activation.store_path,
            .ready = activation.ready,
            .attached_pids = try allocator.dupe(SerializableAttachedPid, serializable_pids),
        };
    }
    defer {
        for (serializable_activations) |sa| {
            allocator.free(sa.attached_pids);
        }
    }
    
    const serializable_data = SerializableActivations{
        .version = data.version,
        .activations = serializable_activations,
    };
    
    return try std.json.stringifyAlloc(allocator, serializable_data, .{});
}

pub fn deserializeActivations(allocator: Allocator, json_str: []const u8) !activations.Activations {
    // Define structures that match the JSON format for direct parsing
    const JsonAttachedPid = struct {
        pid: i32,
        expiration: ?i64,
    };
    
    const JsonActivation = struct {
        id: []const u8,
        store_path: []const u8,
        ready: bool,
        attached_pids: []JsonAttachedPid,
    };
    
    const JsonActivations = struct {
        version: u8,
        activations: []JsonActivation,
    };
    
    // Parse JSON directly into structures
    const parsed = std.json.parseFromSlice(JsonActivations, allocator, json_str, .{
        .allocate = .alloc_always,
    }) catch |err| switch (err) {
        error.SyntaxError => return activations.Activations.init(allocator),
        else => return err,
    };
    defer parsed.deinit();
    
    var result = activations.Activations.init(allocator);
    result.version = parsed.value.version;
    
    // Convert to internal format
    if (parsed.value.activations.len > 0) {
        result.activations = try allocator.alloc(activations.Activation, parsed.value.activations.len);
        
        for (parsed.value.activations, 0..) |json_activation, i| {
            // Convert attached_pids
            var attached_pids = try allocator.alloc(activations.AttachedPid, json_activation.attached_pids.len);
            for (json_activation.attached_pids, 0..) |json_pid, j| {
                attached_pids[j] = activations.AttachedPid{
                    .pid = json_pid.pid,
                    .expiration = json_pid.expiration,
                };
            }
            
            result.activations[i] = activations.Activation{
                .id = try allocator.dupe(u8, json_activation.id),
                .store_path = try allocator.dupe(u8, json_activation.store_path),
                .ready = json_activation.ready,
                .attached_pids = attached_pids,
            };
        }
    }
    
    return result;
}

// Tests for JSON functionality
test "JSON: serialize empty activations" {
    const allocator = std.testing.allocator;
    
    const data = activations.Activations.init(allocator);
    const json = try serializeActivations(allocator, &data);
    defer allocator.free(json);
    
    try std.testing.expect(std.mem.indexOf(u8, json, "\"version\":1") != null);
    try std.testing.expect(std.mem.indexOf(u8, json, "\"activations\":[]") != null);
}

test "JSON: deserialize empty activations" {
    const allocator = std.testing.allocator;
    
    const json_str = "{\"version\":1,\"activations\":[]}";
    var data = try deserializeActivations(allocator, json_str);
    defer data.deinit(allocator);
    
    try std.testing.expect(data.version == 1);
    try std.testing.expect(data.isEmpty());
}