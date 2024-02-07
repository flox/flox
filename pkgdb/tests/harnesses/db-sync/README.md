# DB Locking Harness

The purpose of this harness is to create workflows which ensure multiple
`pkgdb` processes, especially those using `pkgdb scrape` internaly, can be run
in parallel.

Additionally we try to create situations where `pkgdb scrape` dies and a later
invocation of `pkgdb` attempts to recover.
