# Member reached by descending the `python3Packages` namespace directory.
# It references a catalog input directly and depends on a sibling package
# `helper-lib`, whose refs must also be pulled into the closure. The sibling is
# resolved against the package-set root, not this member's directory.
{ catalogs, helper-lib }: catalogs.myorg.widget-src
