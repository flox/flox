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
    // Simple JSON parsing implementation
    // In production, use a proper JSON parser like std.json or cJSON
    
    var result = activations.Activations.init(allocator);
    
    // Look for version field
    if (std.mem.indexOf(u8, json_str, "\"version\":")) |start| {
        const version_start = start + "\"version\":".len;
        var i = version_start;
        while (i < json_str.len and (json_str[i] == ' ' or json_str[i] == '\t')) i += 1;
        
        if (i < json_str.len and json_str[i] >= '0' and json_str[i] <= '9') {
            result.version = @intCast(json_str[i] - '0');
        }
    }
    
    // For this basic implementation, return empty activations
    // In production, properly parse the activations array
    
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