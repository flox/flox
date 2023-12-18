import sys
import subprocess
from subprocess import Popen
from time import perf_counter
from pprint import pprint
from pathlib import Path

CACHE_DIR = Path.home() / ".cache/flox/pkgdb-v2"


def show_output(p):
    (stdout, stderr) = p.communicate()
    print(f"stdout: {stdout}", file=sys.stderr)
    print(f"stderr: {stderr}", file=sys.stderr)


def main():
    # Make sure we're using the latest changes
    print("Rebuilding pkgdb...", file=sys.stderr)
    subprocess.run(["just", "build-pkgdb"], capture_output=True)
    iterations = 10
    all_times = dict()
    prefixes = [
        "python310Packages",
        "python311Packages",
        "rubyPackages",
        "nodePackages",
        "linuxKernel",
    ]
    scrape_args = [
        "pkgdb",
        "scrape",
        "github:NixOS/nixpkgs/release-23.05",
        "legacyPackages",
        "aarch64-darwin",
    ]
    # Scrape one at a time
    durations = list()
    for i in range(iterations):
        print(f"iteration {i}", file=sys.stderr)
        for item in CACHE_DIR.iterdir():
            item.unlink()
        start = perf_counter()
        for prefix in prefixes:
            proc = Popen(scrape_args + [prefix], stdout=subprocess.PIPE)
            proc.wait()
            if proc.returncode != 0:
                print("==== Failed ====", file=sys.stderr)
                show_output(proc)
                return
        stop = perf_counter()
        durations.append(stop - start)
    all_times["serial"] = sum(durations) / iterations
    pprint(all_times)
    # Scrape in parallel
    durations = list()
    for _ in range(iterations):
        print(f"iteration {i}", file=sys.stderr)
        for item in CACHE_DIR.iterdir():
            item.unlink()
        procs = list()
        start = perf_counter()
        for prefix in prefixes:
            proc = Popen(scrape_args + [prefix], stdout=subprocess.PIPE)
            procs.append(proc)
        for p in procs:
            p.wait()
            if p.returncode != 0:
                print("==== Failed ====", file=sys.stderr)
                show_output(proc)
                return
        stop = perf_counter()
        durations.append(stop - start)
    all_times["parallel"] = sum(durations) / iterations
    pprint(all_times)


if __name__ == "__main__":
    main()
