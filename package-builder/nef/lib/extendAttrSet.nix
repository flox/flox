{ lib, nef }:
{
  /*
    Apply an overlay function to a package set, choosing
    `overrideScope` (sets created with `makeScope`) or `extend` (sets
    created with `makeExtensible`, e.g. `nixpkgs#beamPackages`, or
    anything else providing its own `extend` with the same semantics).
    Shared by `extendAttrSet` below and by `lib.nef.instantiate`'s
    deep-override overlay builder, which applies its own overlay to
    `nixpkgs` (or a nested package set) the same way -- both need the
    one dispatch nixpkgs package sets support, not two copies of it.

    # Type

    ```
    applyOverlay :: Attrs -> (Attrs -> Attrs -> Attrs) -> [ String ] -> Attrs
    ```
  */
  applyOverlay =
    packageSet: overlay: attrPath:
    if packageSet ? overrideScope then
      packageSet.overrideScope overlay
    else if packageSet ? extend then
      packageSet.extend overlay
    else
      throw ''
        Cannot extend '${lib.showAttrPath attrPath}', since it is not a supported package set.
        Package sets must be attrsets created with `makeScope` or `makeExtensible`.
      '';

  /*
    Extend a package set, i.e. an attrset defined
    via either `makeExtensible`[1] or `makeScope`[2].
    - create an overlay for the current attrset via `mkOverlay`.
    - override the attrset via either `overrideScope` or `extend`
      (via `applyOverlay` above).

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
    in
    builtins.addErrorContext "while extending package set '${lib.showAttrPath attrPath}'" (
      nef.applyOverlay packageSet overlay attrPath
    );
}
