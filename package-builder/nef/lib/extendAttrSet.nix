{ lib, nef }:
{
  /*
    Extend a package set, i.e. an attrsetr defined
    via either `makeExtensible` or `makeScope`.
    - create an overlay for the current attrset via `mkOverlay`.
    - override the attrset via either `overrideScope` or `extend`.

    If a non-package-set attrset is passed we thow an error,
    as the attrset is likely an output attribute e.g. of `mkDerivation`.

    TODO: We might debate whether it makes sense to wrap `overrideAttrs` in the same way here.
  */
  extendAttrSet =
    attrPath: currentScope: packageSet: extensions:
    let
      overlay = nef.mkOverlay attrPath currentScope extensions;
      extendedAttrSet =
        if packageSet ? overrideScope then
          packageSet.overrideScope overlay
        else if packageSet ? extend then
          packageSet.extend overlay
        else
          throw "dont know how to extend ${lib.showAttrPath attrPath}";
    in
    extendedAttrSet;
}
