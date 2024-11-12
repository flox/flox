# Garbage Collection

`pkgdb` has a garbage collector command `pkgdb gc` which uses a 30-day timer to
mark existing databases as _stale_ to be removed from the user's system.

In the future we may have additional methods of detecting __staleness_ but for
now, this simple approach is suitable.


## Staleness

The 30-day timer can be modified using the option `-a,--min-age AGE` where
`AGE` is _number of days_.

The flag `--dry-run` may be used to list stale databases without actually
deleting them.

### Future Work: Staleness

- Use list of projects on a user's system to detect databases which should
  be preserved.
  + Look in their lockfiles on `main` and ( possibly ) any _active_ branches
    to see which databases are still in use.
    
    
## Triggering Garbage Collection

Today triggering garbage collection is a manual process.
In the future we may implicitly trigger _gc_ as background tasks when various
`pkgdb` operations are performed.

### Future Work: Triggers

- Trigger _gc_ when `flox upgrade` is run on a project.
- Trigger _gc_ when `flox update` is run in either a project or on the
  _global registry_.
- Add this as a part of a `flox` daemon service to be run on a timer.
  + hourly, nightly, weekly, monthly, etc.


## Interactions with `flox` CLI

Today we have a `flox gc` command which triggers the equivalent of `nix gc`.
We _could_ extend this existing routine to also invoke `pkgdb gc`.
Other than that we do not have any integration _today_.

### Future Work: CLI

- Implicitly trigger `pkgdb gc` for operations described in
  [Future Work: Triggers](#future-work--triggers).
