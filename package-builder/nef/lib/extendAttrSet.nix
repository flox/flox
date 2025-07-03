{ lib, nef }:
{
  /*
    Extend a package set, i.e. an attrset defined
    via either `makeExtensible`[1] or `makeScope`[2].
    - create an overlay for the current attrset via `mkOverlay`.
    - override the attrset via either `overrideScope` or `extend`.

    If a non-package-set attrset is passed we thow an error,
    as the attrset is likely an output attribute e.g. of `mkDerivation`.

    TODO: We might debate whether it makes sense to wrap `overrideAttrs` in the same way here.

    [1]: <https://noogle.dev/f/lib/makeExtensible>
    [2]: <https://noogle.dev/f/lib/makeScope>

    # Type

    ```
    extendAttrSet :: [ String ] -> Attrs -> Attrs -> Attrs -> Attrs

    # Arguments

    `attrPath`
    : Current attrPath of the set is extended, used for messaging

    `currentScope`
    : Current scope, i.e. the union of all parent attr sets.
      Used as a fallback by `nef.mkOverlay`.

    `packageSet`
    : The value at `attrPath`, required to be a package set,
      i.e. defined via either `makeExtensible`[1] or `makeScope`[2].

    `extensions`
    : The extensions structure for the current attrPath,
      a Directory value produced by nef.dirToAttrs
  */
  extendAttrSet =

    attrPath: currentScope: packageSet: extensions:
    let
      overlay = nef.mkOverlay attrPath currentScope extensions;
      extendedAttrSet =
        # For package sets created with `makeScope`,
        # use `overrideScope` to apply the overlay to the packageSet.
        if packageSet ? overrideScope then
          packageSet.overrideScope overlay
        # Some package sets, like `nixpkgs#beamPackages`, use `makeExtensible`,
        # which provides an `extend` function to apply an overlay to the
        # "extensible" packageSet.
        # There may be other sets that define their own variant of `extend`,
        # in that case we assume it shares the same semantics as the result
        # of `makeExtensible`.
        else if packageSet ? extend then
          packageSet.extend overlay
        else
          throw ''
            Cannot extend '${lib.showAttrPath attrPath}', since it is not a supported package set.
            Package sets must be attrsets created with `makeScope` or `makeExtensible`.
          '';
    in
    extendedAttrSet;
}
