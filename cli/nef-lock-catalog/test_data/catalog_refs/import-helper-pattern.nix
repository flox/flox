# Helper destructuring the catalog namespace into per-org attrsets.
{ myorg, ... }: myorg.toolkit.readVersion
