# Must receive the other source's deep override of
# `topLevelDependency`, not this repository's stub for it.
{ topLevelDependency, ... }: "depShadowsOverrideTarget sees ${topLevelDependency}"
