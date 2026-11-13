---
name: changing-ld-floxlib
description: Use when changing files in ld-floxlib/.
---

## ld-floxlib (LD_AUDIT shared library)

`ld-floxlib/ld-floxlib.c` is a C shared library loaded via
`LD_AUDIT` to resolve dynamic libraries from Flox environments.

### GLIBC version binding requirement

**Every libc function used in this file MUST have an explicit
`.symver` asm binding** in both architecture blocks:
- `__aarch64__`: `GLIBC_2.17`
- `__x86_64__`: `GLIBC_2.2.5`

When adding or changing any C standard library call (`malloc`,
`strlen`, `strdup`, etc.), add the corresponding `__asm__`
statement in both `#if` blocks at the top of the file. Missing
bindings cause the library to link against a newer GLIBC version
than the target host supports, breaking portability.

### Build and test

```
cd ld-floxlib && make clean && make && make test
```
