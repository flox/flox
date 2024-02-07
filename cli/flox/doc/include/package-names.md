## Package names
Packages are organized in a hierarchical structure such that certain packages
are found at the top level (e.g. `ripgrep`),
and other packages are found under package sets (e.g. `python310Packages.pip`).

The full name of a package will be something like
`legacyPackages.x86_64-linux.python310Packages.pip`
and is referred to as its "attribute path"
(note that "legacyPackages" has nothing to do with packages being out of date).
This attribute path is a sequence of attributes
(e.g. "python310Packages" and "pip") joined by a delimiter (".").
For many use cases, the leading `legacyPackages.<system>` is left off.
The remaining portion of the attribute path (e.g. `python310Packages.pip`) is
referred to as the "relative path" or "path" for short.

This is the portion that is searched when you execute a `flox search` command.
The path is also the portion shown by `flox show`.
Finally, the path appears in your manifest after a `flox install`.

```toml
[install]
ripgrep.path = "ripgrep"
pip.path = "python310Packages.pip"
```
