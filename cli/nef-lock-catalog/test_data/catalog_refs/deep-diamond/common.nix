# Shared helper referencing a package through whichever namespace was
# forwarded down the chain.
{ ns }: ns.somepkg.someattr
