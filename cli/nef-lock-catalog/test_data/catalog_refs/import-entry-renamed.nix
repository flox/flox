# Pattern: the catalog root is forwarded to an import under a different name.
# The imported file's refs are rooted at `cats` and must be rewritten back to
# the parent's `catalogs` root.
{ catalogs }:
import ./import-helper-renamed.nix { cats = catalogs; }
