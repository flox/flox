# test_data

See [`cli/mk_data/`](cli/mk_data/) for how this is used.

Changes to the mocks won't be picked up by the tests until they are `git` staged and the `nix develop` shell is re-evaluated. Otherwise you'll get errors like this:

```console
❌ ERROR: path to mock data file doesn't exist: /nix/store/hdlchirs5p2cq5fvsv32j65h90hibn3p-generated/search/java_suggestions.json
```
