# Shadows the colliding global override at the same attr path with the
# consumer's own definition. It never references `prev`, so nothing
# reaches the base's poisoned entry for this name.
{ }: "consumer's own topLevelDependency"
