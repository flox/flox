# Using Valgrind for Memory Profiling

## Overview

`valgrind` is a memory profiling toolkit that is available in our development
shell on **Linux only**[^1].

[^1]: `valgrind` may work on OSX, but I have not personally had success with it
      which is why it has not been included in the Darwin development shell.

While `valgrind` has a variety of tools, we will focus on `memcheck` and 
`massif` in this document.

Please note that most `valgrind` tools work best when you build executables with
additional debug info, so prefer `make check DEBUG=1` which adds `-ggdb3` and
a few other debug metadata options.


### Default Flags

Default options for `valgrind` can be found in `pkgdb/.valgrindrc` and a default
suppressions file can be found in `pkgdb/build-aux/valgrind.supp`.


### Caches

When profiling consumption try to keep in mind how existing caches for `nix` and
`pkgdb` may effect your results.

You may find it useful to set a temporary `XDG_CACHE_HOME` or to delete
`~/.cache/{flox,nix}` before each profiling run.


### memcheck: Leak Checker

This tool monitors a running program and attempts to identify potential 
memory leaks.

Leaks are categorized as being "definitely lost", "possibly lost",
or "still reachable [at exit]".
We are primarily interested in "definitely lost" and _large blocks_ of
"still reachable" memory.


#### Quick Start

```shell
$ nix develop; # only if you aren't using direnv
$ cd pkgdb;
$ make clean;
$ make -j DEBUG=1;
$ rm -rf ~/.cache/{flox,nix};  # Optional
$ valgrind --tool=memcheck ./bin/pkgdb <SUB-COMMAND> [ARGS...];
```


#### Which info to prioritize

Focus first on large blocks _definitely lost_ bytes.
Ignore small allocations ~64 bytes, or at the very least don't dedicate much
time to eliminating them.

Next up focus on large blocks of _still reachable_ memory that is never
explicitly freed at runtime.
While these are tracked ( not technically leaked ), they do waste space if they
persist after they're done being used.
Some allocations such as those made by the `nix` evaluator are expected to
persist until `exit` and can be ignored.

Next you have _possibly lost_ allocations.
In my experience these most commonly appear around custom allocators or external
libraries and do not truly reflect memory leaks, but rather gaps in `valgrind`'s
ability to analyze a program.
If these are large it may be worthwhile to investigate them, but in my
experience these are often benign.

There are a variety of other checks, often not explicitly related to _leaks_,
that will be reported by `memcheck` that we will not discuss in this guide.


#### Ignores ( Suppressions )

You can explicitly mark some reports by `memcheck` as being benign/ignored
using a suppression file.

We will defer to the `valgrind` manual on [Writing Suppression Files][1] for
details, but do want to note that we store our suppression files in
[pkgdb/build-aux/valgrind.supp](../build-aux/valgrind.supp).
Feel free to add additional suppression files and include them
from `valgrind.supp`.

[1]: https://valgrind.org/docs/manual/mc-manual.html#mc-manual.suppfiles

For non-leak related warnings, especially "conditional jump depends on..."
messages you can often ignore these unless you're debugging 
segmentation faults or undefined behavior.


### massif: Heap Analyzer

Want to find out what parts of your code-base are hogging memory?
`massif` is the tool for you!


`massif` will collect detailed heap snapshots as your program runs which can
be analyzed by a variety of tools, most commonly `ms_print` and
`massif-visualizer`, to identify which code-paths are consuming large amounts
of memory, and when.


#### Quick Start

```shell
$ nix develop; # only if you aren't using direnv
$ cd pkgdb;
$ make clean;
$ make -j DEBUG=1;
$ rm -rf ~/.cache/{flox,nix};  # Optional
$ valgrind --tool=massif ./bin/pkgdb <SUB-COMMAND> [ARGS...];
$ massif-viewer ./massif.out.*;
```


#### Suppressions

Suppressions in `build-aux/valgrind.supp` are not used by default in
`massif` so that we can track memory consumption by `nix`
evaluators and parsers.

You can enable the use of suppressions with
`valgrind --tool=massif --suppressions=./build-aux/valgrind.supp ...`.


#### massif Visualizer

`massif-visualizer` is a fancy `massif.out.*` file viewer that I
strongly recommend.

It has been made available in the development shell for Linux users.
