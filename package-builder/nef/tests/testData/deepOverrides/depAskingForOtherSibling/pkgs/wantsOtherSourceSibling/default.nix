# Asks for `ownSibling` by name, the same name `depWithOwnSibling`
# stubs for its own override -- but this repository has no
# `ownSibling` of its own. If sibling stubs were still shared across
# every override-bearing source (rather than scoped to each one's own
# source), this would be stubbed too, and would throw the same way
# `depWithOwnSibling`'s own override does. It isn't: the default below
# is used.
{
  ownSibling ? "source B does not see source A's sibling",
  ...
}:
ownSibling
