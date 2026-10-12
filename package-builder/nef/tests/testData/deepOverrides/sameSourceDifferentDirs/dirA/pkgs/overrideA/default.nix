# `topLevelValue` is a shallow package here and also exists in the
# base, so its stub warns and returns the base value.
{ topLevelValue, ... }: "dirA override sees ${topLevelValue}"
