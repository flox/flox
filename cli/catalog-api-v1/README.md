# Catalog API Client

A rust client library for the Flox Catalog API.
The client is generated from a (currently vendored) [OpenAPI Spec](./openapi.json)
using the [progenitor](https://github.com/oxidecomputer/progenitor) crate.

## Documentation

The types and methods of the client are docuemnted using rustdoc.
[progenitor](https://github.com/oxidecomputer/progenitor) tries it's best
to enhance the documentation of the generated client based on the spec.

```
$ cargo doc --open -p catalog-api-v1
```

## Updating the Client

Since the client code is generated directly into [the source directory](./src/),
changes to the [OpenAPI Spec](./openapi.json) are reflected after the next code analysis.
It may be necessary to updated the shims in [`flox-rust-sdk`](../flox-rust-sdk/)
after the client was updated.

```
# change openapi.json
$ cargo check -p catalog-api-v1
```

Finally, remember to check-in the updated sources.
Mid-term updating the client library and proposing it as a PR will be done by automation.

## Debugging

[mitmproxy](https://mitmproxy.org/) can be used to debug requests and responses from the Catalog API.

1. Start the interface and leave it running in a separate terminal:
    ```
    nix run github:flox/nixpkgs/stable.20240203#mitmproxy
    ```
    Note that `mitmproxy` has been broken on Darwin for some time now,
    hence the need to run it from an old nixpkgs revision.
    Of course once we implement catalog binary cache verification
    then using this will be as simple as `flox install mitmproxy`,
    but in the meantime we just run it from the most recent stable
    nixpkgs revision on which it is known to build.
1. Install the Certificate Authority per [these instructions](https://docs.mitmproxy.org/stable/concepts-certificates/).
1. Run a `flox` command, using the catalog and the proxy:

        HTTPS_PROXY=http://localhost:8080 SSL_CERT_FILE=~/.mitmproxy/mitmproxy-ca-cert.pem flox show bash

1. Explore the recorded flows in the `mitmproxy` interface.
