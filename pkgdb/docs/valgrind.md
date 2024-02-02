# Using Valgrind for Memory Profiling

## Overview

`valgrind` is a memory profiling toolkit that is available in our development
shell on **Linux only**[^1].

[^1]: `valgrind` may work on OSX but I have not personally had success with it
      which is why it has not been included in the Darwin development shell.

While `valgrind` has a variety of tools, we will focus on `memcheck` and 
`massif` in this document.

Please note that most `valgrind` tools work best when you build executables with
additional debug info, so prefer `make check DEBUG=1` which adds `-ggdb3` and
a few other debug metadata options.


### memcheck: Leak Checker

This tool monitors a running program and attempts to identify potential 
memory leaks.

Leaks are categorized as being "definitely lost", "possibly lost",
or "still reachable [at exit]".
We are primarily interested in "definitely lost" and _large blocks_ of
"still reachable" memory.


#### Quick Start

```shell
# TODO
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


<!-- TODO -->


#### Quick Start

```shell
# TODO
```


#### massif Visualizer

<!-- TODO -->
