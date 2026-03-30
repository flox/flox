# Catalog API Client

A rust client library for the Flox Catalog API.
The client is generated from a (currently vendored) [OpenAPI Spec](./openapi.json)
using the [progenitor](https://github.com/oxidecomputer/progenitor) crate.

## Documentation

The types and methods of the client are documented using rustdoc.
[progenitor](https://github.com/oxidecomputer/progenitor) tries it's best
to enhance the documentation of the generated client based on the spec.

```
$ cargo doc --open -p catalog-api-v1
```

## Updating the Client

Since the client code is generated directly into [the source directory](./src/),
changes to the [OpenAPI Spec](./openapi.json) are reflected after the next code analysis.
It may be necessary to update the shims in [`flox-rust-sdk`](../flox-rust-sdk/)
after the client was updated.

```
# change openapi.json
$ cargo check -p catalog-api-v1
```

Finally, remember to check in the updated sources.
Mid-term updating the client library and proposing it as a PR will be done by automation.

## Debugging

[mitmproxy](https://mitmproxy.org/) can be used to debug requests and responses from the Catalog API.

1. Start the interface in one terminal:

        flox activate -r dcarley/mitmproxy -- mitmproxy

1. Proxy a `flox` command in another terminal:

        % flox activate -r dcarley/mitmproxy
        ✔ Attached to existing activation of environment 'dcarley/mitmproxy (local)'
        To stop using this environment, type 'exit'

        Start the proxy in one terminal:
          mitmproxy    (TUI)
          mitmweb      (web interface)
          mitmdump     (non-interactive)

        Then route commands through it with:
          proxy <command>
          proxy curl https://example.com

        % proxy flox show bash
        …

1. Inspect the recorded flows in the first terminal.
