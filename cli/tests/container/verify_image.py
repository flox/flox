#!/usr/bin/env python3
"""Check containerize's activation data and account files from image bytes only.

This reads the Docker archive emitted by streamLayeredImage. It never extracts
an image or follows a link into the build machine's /nix/store. Only the handful
of files under test are retained while layer payloads are streamed once.
"""

import argparse
import json
import posixpath
import sys
import tarfile
from dataclasses import dataclass
from pathlib import Path


class InvalidImage(ValueError):
    """The archive does not satisfy the containerize image contract."""


def require(condition, message):
    if not condition:
        raise InvalidImage(message)


def image_path(name):
    require(isinstance(name, str) and bool(name), f"Invalid image path: {name!r}")
    require(".." not in name.split("/"), f"Parent traversal in image path: {name!r}")
    return posixpath.normpath(name).lstrip("/").removeprefix("./")


def payload(archive, member):
    require(member.isreg(), f"{member.name}: expected a regular file")
    require(member.size <= 1024 * 1024, f"{member.name}: unexpectedly large test payload")
    with archive.extractfile(member) as source:
        return source.read()


@dataclass
class Entry:
    header: tarfile.TarInfo
    data: bytes | None


def selected_rootfs(archive, layers, wanted):
    """Fold selected paths and their ancestors in manifest order, not tar order."""
    watched = set(wanted)
    for name in wanted:
        parent = posixpath.dirname(name)
        while parent:
            watched.add(parent)
            parent = posixpath.dirname(parent)
    entries = {}

    def remove_tree(name, children_only=False):
        for existing in list(entries):
            if (not children_only and existing == name) or existing.startswith(name + "/"):
                del entries[existing]

    for layer_name in layers:
        layer_member = archive.getmember(layer_name)
        require(layer_member.isreg(), f"{layer_name}: expected a regular layer tar")
        added, removed = {}, []
        with archive.extractfile(layer_member) as source:
            with tarfile.open(fileobj=source, mode="r|*") as layer:
                for member in layer:
                    name = image_path(member.name)
                    parent, basename = posixpath.split(name)
                    if basename == ".wh..wh..opq":
                        removed.append((parent, True))
                    elif basename.startswith(".wh."):
                        removed.append((posixpath.join(parent, basename[4:]), False))
                    elif name in watched:
                        data = payload(layer, member) if name in wanted and member.isreg() else None
                        added[name] = Entry(member, data)
        # Whiteouts remove lower-layer entries, never entries from their own layer.
        for name, children_only in removed:
            if not name:
                entries.clear()
            else:
                remove_tree(name, children_only)
        for name in sorted(added, key=lambda value: value.count("/")):
            if not added[name].header.isdir():
                remove_tree(name, children_only=True)
            entries[name] = added[name]
    return entries


def checked_entry(entries, name, directory=False):
    require(name in entries, f"Image is missing {name}")
    parent = posixpath.dirname(name)
    while parent:
        if parent in entries:
            require(entries[parent].header.isdir(), f"{parent}: expected a real directory")
        parent = posixpath.dirname(parent)
    entry = entries[name]
    require(entry.header.isdir() if directory else entry.header.isreg(),
            f"{name}: expected a {'directory' if directory else 'regular file'}, "
            f"got tar type {entry.header.type!r} (link {entry.header.linkname!r})")
    return entry


def verify_image(path, *, cuda, mode, user="", passwd_lines=(), group_lines=(),
                 uid=0, gid=0, working_dir=None):
    with tarfile.open(path, mode="r:*") as image:
        manifest = json.loads(payload(image, image.getmember("manifest.json")))
        require(isinstance(manifest, list) and len(manifest) == 1,
                "Expected a single-image Docker archive")
        config = json.loads(payload(image, image.getmember(manifest[0]["Config"])))
        config = config["config"]
        require(config.get("User", "") == user,
                f"User: expected {user!r}, got {config.get('User', '')!r}")
        entrypoint = config["Entrypoint"]
        require(isinstance(entrypoint, list) and len(entrypoint) == 4
                and entrypoint[1:3] == ["activate", "--activate-data"],
                f"Unexpected activation entrypoint: {entrypoint!r}")
        context_path = image_path(entrypoint[3])
        wanted = {context_path, "etc/passwd", "etc/group", "tmp", "run", "run/flox"}
        if working_dir:
            require(config.get("WorkingDir") == working_dir, "WorkingDir was not preserved")
            wanted.add(image_path(working_dir))
        layers = manifest[0]["Layers"]
        require(isinstance(layers, list) and layers, "Expected image layers")
        entries = selected_rootfs(image, layers, wanted)

    context = json.loads(checked_entry(entries, context_path).data)
    require(context["attach_ctx"]["flox_env_cuda_detection"] == cuda,
            f"CUDA gate: expected {cuda!r}, got "
            f"{context['attach_ctx']['flox_env_cuda_detection']!r}")
    require(context["mode"] == mode, f"Activation mode: expected {mode!r}, got {context['mode']!r}")
    require(entrypoint[0] == context["flox_activate_store_path"] + "/libexec/flox-activations",
            "Entrypoint and activation context select different environments")

    accounts = {
        "etc/passwd": ["root:x:0:0:root user:/var/empty:/bin/sh", *passwd_lines,
                       "nobody:x:65534:65534:nobody:/var/empty:/bin/sh"],
        "etc/group": ["root:x:0:", *group_lines, "nobody:x:65534:"],
    }
    for name, expected in accounts.items():
        entry = checked_entry(entries, name)
        actual = entry.data.decode("utf-8").splitlines()
        require(actual == expected, f"{name}: expected {expected!r}, got {actual!r}")
        require(entry.header.mode & 0o444 == 0o444, f"{name}: not readable by every image user")
        require((entry.header.uid, entry.header.gid) == (0, 0), f"{name}: expected root ownership")

    directories = {"tmp": (0o1777, 0, 0), "run": (0o1770, uid, gid), "run/flox": (0o1770, uid, gid)}
    if working_dir:
        directories[image_path(working_dir)] = (0o755, uid, gid)
    for name, expected in directories.items():
        header = checked_entry(entries, name, directory=True).header
        actual = (header.mode, header.uid, header.gid)
        require(actual == expected, f"{name}: expected mode/uid/gid {expected!r}, got {actual!r}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("image", type=Path)
    parser.add_argument("--cuda", required=True, choices=["0", "1"])
    parser.add_argument("--mode", required=True, choices=["dev", "run"])
    parser.add_argument("--user", default="")
    parser.add_argument("--passwd-line", action="append", default=[])
    parser.add_argument("--group-line", action="append", default=[])
    parser.add_argument("--uid", type=int, default=0)
    parser.add_argument("--gid", type=int, default=0)
    parser.add_argument("--working-dir")
    args = parser.parse_args()
    try:
        verify_image(args.image, cuda=args.cuda, mode=args.mode, user=args.user,
                     passwd_lines=args.passwd_line, group_lines=args.group_line,
                     uid=args.uid, gid=args.gid, working_dir=args.working_dir)
    except (InvalidImage, json.JSONDecodeError, KeyError, TypeError, UnicodeError, OSError, tarfile.TarError) as error:
        print(f"FAIL: {args.image}: {error}", file=sys.stderr)
        return 1
    print(f"PASS: {args.image.name}: CUDA={args.cuda}, mode={args.mode}, regular account files")
    return 0


if __name__ == "__main__":
    sys.exit(main())
