# Top-level alias package.
# Re-exports the `isdr-zk-client` member of the in-repo `python3Packages`
# package set. This is the deep-overlay form NEF repos use for same-repo
# references: the dependency is a lambda argument injected from the extended
# package scope, and the member is reached with a select (`python3Packages.<member>`).
#
# The alias takes no `catalogs` argument of its own. Its catalog inputs come
# entirely from the member it points at, so scanning the alias must follow
# `python3Packages.isdr-zk-client` to that member and surface its refs.
{ python3Packages }: python3Packages.isdr-zk-client
