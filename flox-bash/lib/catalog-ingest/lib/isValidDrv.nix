{lib}:
# checks whether a given record is a valid derivation
# limited to catchable errors (paritcularly not: missing attributes, aborts)
d: let
  r = builtins.tryEval (
    let
      eval = [
        (lib.isDerivation d) # element should be a derivation
        (!(lib.attrByPath ["meta" "broken"] false d)) # element should not be broken
        (d ? name) # element has a name *
        (builtins.seq d.outputs true)
      ];
    in
      builtins.all lib.id eval
  );
in (r.success && r.value)
