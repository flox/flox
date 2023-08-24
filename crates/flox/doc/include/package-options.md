## Package Options

Flox package arguments are specified as a tuple of
stability, channel, name, and version in the format:
`<stability>`.`<channel>`.`<name>`@`<version>`

The version field is optional, defaulting to the latest version if not specified.

The stability field is also optional, defaulting to "stable" if not specified.

The channel field is also optional, defaulting to "nixpkgs-flox" if not specified,
_but only if using the "stable" stability_. If using anything other than the
default "stable" stability, the channel *must* be specified.

For example, each of the following will install the latest hello version 2.12 from
the stable channel:
```
flox install stable.nixpkgs-flox.hello@2.12
flox install stable.nixpkgs-flox.hello
flox install nixpkgs-flox.hello@2.12
flox install nixpkgs-flox.hello
flox install hello@2.12
flox install hello
```

... and each of the following will install the older hello version 2.10
from the stable channel:
```
flox install stable.nixpkgs-flox.hello@2.10
flox install nixpkgs-flox.hello@2.10
flox install hello@2.10
```

... but only the following will install the older hello version 2.10 from the unstable channel:
```
flox install unstable.nixpkgs-flox.hello@2.10
```
