## Package names
Packages are organized in a hierarchical structure such that certain packages
are found at the top level (e.g. `ripgrep`),
and other packages are found under package sets (e.g. `python310Packages.pip`).
We call this location within the catalog the "pkg-path".

The pkg-path is searched when you execute a `flox search` command.
The pkg-path is what's shown by `flox show`.
Finally, the pkg-path appears in your manifest after a `flox install`.

```toml
[install]
ripgrep.pkg-path = "ripgrep"
pip.pkg-path = "python310Packages.pip"
```
