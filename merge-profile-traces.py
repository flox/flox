#!/usr/bin/env python3
"""Merge flox profile trace files into a single Chrome Trace JSON file.

Usage:
    merge-profile-traces.py <profile-dir> [-o merged.json]

Reads all trace files from the profile directory:
  - flox-cli-*.json       (Chrome Trace JSON from the flox CLI)
  - flox-activations-*.json (Chrome Trace JSON from flox-activations)
  - shell-trace.ndjson    (NDJSON from shell activation scripts)

Outputs a single Chrome Trace JSON file viewable at https://ui.perfetto.dev/
"""

import json
import glob
import sys
import os
import argparse


def read_chrome_trace(path):
    """Read a Chrome Trace JSON file and return its events.

    Handles truncated files (e.g. when the process crashed before flushing)
    by attempting to parse valid JSON objects from the content.
    """
    with open(path) as f:
        content = f.read()

    if not content.strip():
        print(f"Warning: empty file {path}", file=sys.stderr)
        return []

    try:
        data = json.loads(content)
        if isinstance(data, list):
            return data
        elif isinstance(data, dict) and "traceEvents" in data:
            return data["traceEvents"]
        else:
            print(f"Warning: unexpected format in {path}", file=sys.stderr)
            return []
    except json.JSONDecodeError:
        # File may be truncated. Try to recover individual JSON objects.
        # Chrome trace format is either [...] or {"traceEvents": [...]}.
        # tracing-chrome writes one JSON object per line after the opening '['.
        print(f"Warning: truncated JSON in {path}, attempting recovery", file=sys.stderr)
        events = []
        for line in content.splitlines():
            line = line.strip().rstrip(",")
            if line in ("", "[", "]", "{", "}"):
                continue
            try:
                obj = json.loads(line)
                if isinstance(obj, dict) and "ph" in obj:
                    events.append(obj)
            except json.JSONDecodeError:
                continue
        print(f"  Recovered {len(events)} events from {path}", file=sys.stderr)
        return events


def read_ndjson(path):
    """Read a newline-delimited JSON file and return its events."""
    events = []
    with open(path) as f:
        for line in f:
            line = line.strip()
            if line:
                try:
                    events.append(json.loads(line))
                except json.JSONDecodeError as e:
                    print(f"Warning: skipping malformed line in {path}: {e}", file=sys.stderr)
    return events


def make_metadata_event(pid, name):
    """Create a process_name metadata event."""
    return {
        "ph": "M",
        "pid": pid,
        "tid": 0,
        "ts": 0,
        "name": "process_name",
        "args": {"name": name},
    }


def remap_pid(events, new_pid):
    """Remap all pid values in events to new_pid, preserving tid."""
    for event in events:
        event["pid"] = new_pid
    return events


def read_epoch(json_path):
    """Read the .epoch file corresponding to a trace JSON file.

    Returns the wall-clock epoch in microseconds, or None if not found.
    The .epoch file records the wall-clock time (since UNIX epoch, in us)
    when the chrome tracing layer was created. Since tracing-chrome uses
    monotonic Instant::now() internally, trace timestamps are relative to
    layer creation. The epoch lets us convert to absolute time.
    """
    epoch_path = json_path.replace(".json", ".epoch")
    if os.path.exists(epoch_path):
        try:
            with open(epoch_path) as f:
                return int(f.read().strip())
        except (ValueError, OSError):
            pass
    return None


def offset_timestamps(events, offset_us):
    """Add offset_us to all event timestamps."""
    for event in events:
        if "ts" in event:
            event["ts"] += offset_us
    return events


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("profile_dir", help="Directory containing profile trace files")
    parser.add_argument("-o", "--output", default=None, help="Output file (default: <profile_dir>/merged.json)")
    args = parser.parse_args()

    profile_dir = args.profile_dir
    output_path = args.output or os.path.join(profile_dir, "merged.json")

    if not os.path.isdir(profile_dir):
        print(f"Error: {profile_dir} is not a directory", file=sys.stderr)
        sys.exit(1)

    merged_events = []
    next_pid = 1

    # Collect all trace files with their epochs for time alignment
    all_trace_files = []  # list of (path, label_prefix)

    cli_files = sorted(glob.glob(os.path.join(profile_dir, "flox-cli-*.json")))
    for path in cli_files:
        orig_pid = os.path.basename(path).replace("flox-cli-", "").replace(".json", "")
        all_trace_files.append((path, f"flox CLI (pid {orig_pid})"))

    activations_files = sorted(glob.glob(os.path.join(profile_dir, "flox-activations-*.json")))
    for path in activations_files:
        orig_pid = os.path.basename(path).replace("flox-activations-", "").replace(".json", "")
        all_trace_files.append((path, f"flox-activations (pid {orig_pid})"))

    # Read epochs from all trace files to find the baseline (earliest start)
    epochs = {}
    for path, _ in all_trace_files:
        epoch = read_epoch(path)
        if epoch is not None:
            epochs[path] = epoch

    baseline_epoch = min(epochs.values()) if epochs else None

    # Process Rust trace files with epoch-based alignment
    for path, label in all_trace_files:
        events = read_chrome_trace(path)
        if not events:
            continue
        pid = next_pid
        next_pid += 1
        remap_pid(events, pid)
        # Offset timestamps so all traces share a common time origin
        if baseline_epoch is not None and path in epochs:
            offset = epochs[path] - baseline_epoch
            offset_timestamps(events, offset)
        merged_events.extend(events)
        merged_events.append(make_metadata_event(pid, label))

    # Process shell traces (already use wall-clock timestamps via date +%s%N,
    # so offset them relative to the baseline too)
    shell_files = sorted(glob.glob(os.path.join(profile_dir, "shell-trace.ndjson")))
    for path in shell_files:
        events = read_ndjson(path)
        if not events:
            continue
        pid = next_pid
        next_pid += 1
        remap_pid(events, pid)
        # Shell timestamps are absolute wall-clock microseconds;
        # offset them to the same baseline as the Rust traces
        if baseline_epoch is not None:
            offset_timestamps(events, -baseline_epoch)
        merged_events.append(make_metadata_event(pid, "shell scripts"))
        merged_events.extend(events)

    if not merged_events:
        print(f"No trace files found in {profile_dir}", file=sys.stderr)
        sys.exit(1)

    # Sort by timestamp (metadata events with ts=0 go first)
    merged_events.sort(key=lambda e: e.get("ts", 0))

    with open(output_path, "w") as f:
        json.dump({"traceEvents": merged_events}, f)

    total_sources = len(cli_files) + len(activations_files) + len(shell_files)
    print(f"Merged {len(merged_events)} events from {total_sources} file(s) -> {output_path}")
    print(f"Open at https://ui.perfetto.dev/")


if __name__ == "__main__":
    main()
