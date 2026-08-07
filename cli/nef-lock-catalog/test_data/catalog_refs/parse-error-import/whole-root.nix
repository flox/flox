# Passes the whole namespace to a helper that does not parse. This reaches the
# import through `top_ident_param`, which would otherwise read the unparsed file
# as a pattern parameter and widen the root to a sentinel.
{ catalogs }:
(import ./broken.nix catalogs).result
