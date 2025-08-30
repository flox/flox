const std = @import("std");
const Allocator = std.mem.Allocator;

const activations = @import("activations.zig");
const Error = @import("main.zig").Error;

// Simple JSON serialization/deserialization for activations
// In production, you'd want to use a more robust JSON library

pub fn serializeActivations(allocator: Allocator, data: *const activations.Activations) ![]u8 {
    var result = std.ArrayList(u8).init(allocator);
    defer result.deinit();
    const writer = result.writer();
    
    try writer.writeAll("{\"version\":");
    try writer.print("{}", .{data.version});
    try writer.writeAll(",\"activations\":[");
    
    for (data.activations, 0..) |activation, i| {
        if (i > 0) try writer.writeAll(",");
        try writer.writeAll("{");
        
        try writer.writeAll("\"id\":\"");
        try writer.writeAll(activation.id);
        try writer.writeAll("\",");
        
        try writer.writeAll("\"store_path\":\"");
        try writer.writeAll(activation.store_path);
        try writer.writeAll("\",");
        
        try writer.print("\"ready\":{},", .{activation.ready});
        
        try writer.writeAll("\"attached_pids\":[");
        for (activation.attached_pids, 0..) |pid, j| {
            if (j > 0) try writer.writeAll(",");
            try writer.writeAll("{");
            try writer.print("\"pid\":{}", .{pid.pid});
            if (pid.expiration) |exp| {
                try writer.print(",\"expiration\":{}", .{exp});
            } else {
                try writer.writeAll(",\"expiration\":null");
            }
            try writer.writeAll("}");
        }
        try writer.writeAll("]");
        
        try writer.writeAll("}");
    }
    
    try writer.writeAll("]}");
    return try allocator.dupe(u8, result.items);
}

pub fn deserializeActivations(allocator: Allocator, json_str: []const u8) !activations.Activations {
    // Use std.json for proper parsing
    const parsed = std.json.parseFromSlice(std.json.Value, allocator, json_str, .{}) catch |err| switch (err) {
        error.SyntaxError => return activations.Activations.init(allocator),
        else => return err,
    };
    defer parsed.deinit();
    
    var result = activations.Activations.init(allocator);
    
    const root = parsed.value.object;
    
    // Parse version
    if (root.get("version")) |version_val| {
        if (version_val == .integer) {
            result.version = @intCast(version_val.integer);
        }
    }
    
    // Parse activations array
    if (root.get("activations")) |activations_val| {
        if (activations_val == .array) {
            const activations_array = activations_val.array;
            
            if (activations_array.items.len > 0) {
                result.activations = try allocator.alloc(activations.Activation, activations_array.items.len);
                
                for (activations_array.items, 0..) |activation_val, i| {
                    if (activation_val != .object) continue;
                    const activation_obj = activation_val.object;
                    
                    // Parse activation fields
                    const id = if (activation_obj.get("id")) |id_val| blk: {
                        if (id_val == .string) {
                            break :blk try allocator.dupe(u8, id_val.string);
                        }
                        break :blk try allocator.dupe(u8, "");
                    } else try allocator.dupe(u8, "");
                    
                    const store_path = if (activation_obj.get("store_path")) |sp_val| blk: {
                        if (sp_val == .string) {
                            break :blk try allocator.dupe(u8, sp_val.string);
                        }
                        break :blk try allocator.dupe(u8, "");
                    } else try allocator.dupe(u8, "");
                    
                    const ready = if (activation_obj.get("ready")) |ready_val| blk: {
                        if (ready_val == .bool) {
                            break :blk ready_val.bool;
                        }
                        break :blk false;
                    } else false;
                    
                    // Parse attached_pids array
                    var attached_pids = std.ArrayList(activations.AttachedPid).init(allocator);
                    defer attached_pids.deinit();
                    
                    if (activation_obj.get("attached_pids")) |pids_val| {
                        if (pids_val == .array) {
                            for (pids_val.array.items) |pid_val| {
                                if (pid_val != .object) continue;
                                const pid_obj = pid_val.object;
                                
                                const pid = if (pid_obj.get("pid")) |p_val| blk: {
                                    if (p_val == .integer) {
                                        break :blk @as(i32, @intCast(p_val.integer));
                                    }
                                    break :blk @as(i32, 0);
                                } else @as(i32, 0);
                                
                                const expiration = if (pid_obj.get("expiration")) |exp_val| blk: {
                                    if (exp_val == .integer) {
                                        break :blk @as(?i64, @as(i64, @intCast(exp_val.integer)));
                                    } else if (exp_val == .null) {
                                        break :blk @as(?i64, null);
                                    }
                                    break :blk @as(?i64, null);
                                } else @as(?i64, null);
                                
                                try attached_pids.append(activations.AttachedPid{
                                    .pid = pid,
                                    .expiration = expiration,
                                });
                            }
                        }
                    }
                    
                    result.activations[i] = activations.Activation{
                        .id = id,
                        .store_path = store_path,
                        .ready = ready,
                        .attached_pids = try attached_pids.toOwnedSlice(),
                    };
                }
            }
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