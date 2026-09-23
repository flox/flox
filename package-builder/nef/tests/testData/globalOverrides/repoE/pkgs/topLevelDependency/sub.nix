# Targets 'topLevelDependency.sub', a path nested under repoA's
# 'topLevelDependency' -- for the prefix-collision test: the two attr
# paths can't coexist in the merged overlay ('topLevelDependency' can't
# be both a package and a directory containing 'sub').
{ }: "overridden by repoE"
