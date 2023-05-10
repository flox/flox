# options that all runners (e.g. bash, eventually containers) should use
{
  config,
  lib,
  ...
}:
with lib; {
  options = {
    environmentVariables = mkOption {
      default = {};
      example = {
        EDITOR = "nvim";
        VISUAL = "nvim";
      };
      description = lib.mdDoc ''
        A set of environment variables. The value of each variable can be either
        a string or a list of strings.  The latter is concatenated, interspersed
        with colon characters.

        Alternatively, this may be a list of sets of environment variables. In
        that case, order of the variables is preserved, and values are not
        escaped, which means variables may be evaluated at runtime.
      '';
      type = with types; let
        envAttrSet =
          # {
          #   FOO = "BAR";
          #   PATH = ["/bin" "/usr/bin"];
          # }
          attrsOf (either str (listOf str));
      in
        either envAttrSet (listOf envAttrSet);
      apply = let
        joinWithColon = envAttrSet:
          mapAttrs (n: v:
            if isList v
            then concatStringsSep ":" v
            else v)
          envAttrSet;
      in
        definition:
          if builtins.isList definition
          then builtins.map joinWithColon definition
          else joinWithColon definition;
    };

    toplevel = mkOption {
      internal = true;
      type = types.package;
    };
    passthru = mkOption {
      description = lib.mdDoc ''
        Packages to expose under toplevel.passthru
      '';
      type = types.attrs;
      internal = true;
    };
  };
}
