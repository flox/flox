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
    max_procs = 10
    iterations = 10
    all_times = dict()
    # Scan over the number of concurrent search processes to see how contention
    # affects the performance
    for n in range(1, max_procs + 1):
        print(f"Running {n} search processes...", file=sys.stderr)
        durations = list()
        # Run `n` concurrent processes `iterations` times so we have decent
        # statistics
        for _ in range(iterations):
            subprocess.run(["pkgdb", "gc", "-a", "0"], capture_output=True)
            for item in CACHE_DIR.iterdir():
                item.unlink()
            procs = list()
            start = perf_counter()
            # Start `n` search processes
            for i in range(n):
                procs.append(
                    Popen(
                        [
                            "pkgdb",
                            "search",
                            "--ga-registry",
                            "-q",
                            "--match-name",
                            "hello",
                        ],
                        stdout=subprocess.PIPE,
                    )
                )
            # Wait for all of the processes to finish
            for p in procs:
                p.wait()
            stop = perf_counter()
            durations.append(stop - start)
            # See if any of them failed, print the error if one did
            for p in procs:
                if p.returncode != 0:
                    print("==== Failed ====", file=sys.stderr)
                    show_output(p)
                    return
        all_times[n] = sum(durations) / iterations
    pprint(all_times)


if __name__ == "__main__":
    main()
