# `otherOwnSibling` is this repository's own unmarked sibling, absent
# from `basePkgs` -- throws, same as `depWithOwnSibling`'s own
# override does for its own sibling. Together with
# `wantsOtherSourceSibling.nix` (which asks for the *other* source's
# sibling by name and does not get stubbed for it), this proves each
# source's stub layer is independently correct, not just "one leaks,
# the other doesn't".
{ otherOwnSibling, ... }: otherOwnSibling
