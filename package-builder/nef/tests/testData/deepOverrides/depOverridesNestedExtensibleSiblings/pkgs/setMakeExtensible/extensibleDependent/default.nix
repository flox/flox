# Must receive the deep override of `extensibleDependency`, not the
# throwing base value.
{ extensibleDependency, ... }: "extensibleDependent sees ${extensibleDependency}"
