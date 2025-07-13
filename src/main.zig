const std = @import("std");
const fs = std.fs;

const Badepo = @import("badepo").Badepo;

pub fn main() !void {
    const alloc = std.heap.page_allocator;

    var zlap = try @import("zlap").Zlap(@embedFile("./commands.zlap")).init(alloc);
    defer zlap.deinit();

    const src_path = zlap.main_args.get("SOURCE").?.value.string;
    const dest_path = zlap.main_args.get("DESTINATION").?.value.string;

    var badepo = try Badepo.init(alloc);
    defer badepo.deinit();

    var source_dir = try fs.cwd().openDir(src_path, .{ .iterate = true });
    defer source_dir.close();
    var walker = try source_dir.walk(alloc);
    defer walker.deinit();

    var total_size: usize = 0;
    while (try walker.next()) |entry| {
        if (entry.kind == .file) {
            const stat = try entry.dir.statFile(entry.basename);
            total_size += stat.size;
        }
    }

    var curr_size: usize = 0;
    try copyDirRecursive(
        alloc,
        src_path,
        dest_path,
        &badepo,
        &curr_size,
        total_size,
    );
}

fn copyDirRecursive(
    allocator: std.mem.Allocator,
    source_path: []const u8,
    dest_path: []const u8,
    badepo: *Badepo,
    curr_size: *usize,
    total_size: usize,
) !void {
    try fs.cwd().makeDir(dest_path);
    var source_dir = try fs.cwd().openDir(source_path, .{ .iterate = true });
    defer source_dir.close();
    var it = source_dir.iterate();
    while (try it.next()) |entry| {
        const source_entry_path = try std.fs.path.join(
            allocator,
            &.{ source_path, entry.name },
        );
        defer allocator.free(source_entry_path);
        const dest_entry_path = try std.fs.path.join(
            allocator,
            &.{ dest_path, entry.name },
        );
        defer allocator.free(dest_entry_path);
        switch (entry.kind) {
            .directory => {
                try copyDirRecursive(
                    allocator,
                    source_entry_path,
                    dest_entry_path,
                    badepo,
                    curr_size,
                    total_size,
                );
            },
            .file => {
                const stat = try fs.cwd().statFile(source_entry_path);
                curr_size.* += stat.size;

                try fs.cwd().copyFile(
                    source_entry_path,
                    fs.cwd(),
                    dest_entry_path,
                    .{},
                );
                // TODO: show filename
                try badepo.print(curr_size.*, total_size, null, .{});
            },
            else => {},
        }
    }
}
