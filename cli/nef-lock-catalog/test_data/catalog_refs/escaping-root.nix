# The whole namespace escapes into an opaque function, widening to a root
# wildcard, which names nothing the server can resolve.
{ catalogs, f }:
f catalogs
