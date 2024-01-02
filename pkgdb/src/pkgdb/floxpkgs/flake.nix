# ============================================================================ #
#
#
#
# ---------------------------------------------------------------------------- #

{

  # -------------------------------------------------------------------------- #

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/release-23.05";

  # -------------------------------------------------------------------------- #

  outputs = { self, nixpkgs, ... }: let

    # ------------------------------------------------------------------------ #

    config = {
      allowUnfree = true;
      allowBroken = true;
    };  # End `config'

    # allowRecursive    = [["legacyPackages" "x86_64-linux" "darwin"]]
    # disallowRecursive = [["legacyPackages" "x86_64-linux" "python3"]]
    # allowPackage      = [["legacyPackages" "x86_64-linux" "python3" "pip"]]
    # disallowPackage   = [["legacyPackages" "x86_64-linux" "gcc"]]
    rules = builtins.fromJSON ( builtins.readFile ./rules.json );

    # ------------------------------------------------------------------------ #

    lib = {
      # setAttrAt attrPath attrSet
      # --------------------------------
      # Get an attribute in `attrSet` at the path specified in `attrPath`.
      #
      # Example:
      #   getAttrAt ["a" "b"] { a = { b = 1; c = 2; }
      #   => 1
      #
      # Type:
      #   setAttrAt :: [String] -> AttrSet -> AttrSet
      getAttrAt = attrPath: attrSet: let
        len       = builtins.length attrPath;
        attrName  = builtins.head attrPath;
        attrValue = builtins.getAttr attrName attrSet;
        subPath   = builtins.tail attrPath;
      in if len == 0 then attrSet else
         if len == 1 then attrValue else
         getAttrAt subPath attrValue;

      # setAttrAt attrPath value attrSet
      # --------------------------------
      # Set an attribute in `attrSet` at the path specified in `attrPath` to
      # the value `value`.
      #
      # Example:
      #   setAttrAt ["a" "b"] 3 { a = { b = 1; c = 2; }
      #   => { a = { b = 3; c = 2; }; }
      #
      # Type:
      #   setAttrAt :: [String] -> Any   -> AttrSet -> AttrSet
      setAttrAt = attrPath: value: attrSet: let
        len       = builtins.length attrPath;
        attrName  = builtins.head attrPath;
        attrValue = attrSet.${attrName} or {};
        subPath   = builtins.tail attrPath;
      in if len == 0 then attrSet else
         if len == 1 then attrSet // { "${attrPath}" = value; } else
          attrSet // {
            ${attrName} = setAttrByPath subPath value attrValue;
          };

      # removeAttrAt attrPath attrSet
      # -----------------------------
      # Remove an attribute in `attrSet` at the path specified in `attrPath`.
      #
      # Example:
      #   removeAttrAt ["a" "b"] 3 { a = { b = 1; c = 2; }
      #   => { a = { c = 2; }; }
      #
      # Type:
      #   setAttrAt :: [String] -> Any   -> AttrSet -> AttrSet
      removeAttrAt = attrPath: attrSet: let
        len       = builtins.length attrPath;
        attrName  = builtins.head attrPath;
        attrValue = builtins.getAttr attrName attrSet;
        subPath   = builtins.tail attrPath;
      in if len == 0 then attrSet else
         if len == 1 then removeAttrs attrSet [attrName] else
         if ! ( builtins.hasAttr attrName attrSet ) then attrSet else
          attrSet // {
            ${attrName} = removeAttrByPath subPath attrValue;
          };

      # eachDefaultSystemMap fn
      # -----------------------
      # Given a function `fn' which takes system names as an argument, produce
      # an attribute set whose keys are system names, and values are the result
      # of applying that system name to `fn'.
      #
      # Example:
      #   eachDefaultSystemMap ( system: "Hello, ${system}!" )
      #   => {
      #     x86_64-linux = "Hello, x86_64-linux!";
      #     aarch64-linux = "Hello, aarch64-linux!";
      #     x86_64-darwin = "Hello, x86_64-darwin!";
      #     aarch64-darwin = "Hello, aarch64-darwin!";
      #   }
      #
      # Type:
      #   eachDefaultSystemMap :: ( String -> Any ) -> AttrSet
      eachDefaultSystemMap = let
        defaultSystems = [
          "x86_64-linux" "aarch64-linux" "x86_64-darwin" "aarch64-darwin"
        ];
      in fn: let
          proc = system: { name = system; value = fn system; };
        in builtins.listToAttrs (map proc defaultSystems);

      # enumeratePaths attrPath
      # -----------------------
      # Given an attribute path, produce a list of attrPaths starting with
      # `attrPath`, followed by it's parent, and so on until the root.
      #
      # Example:
      #   enumeratePaths ["a" "b" "c"]
      #   => [["a" "b" "c"] ["a" "b"] ["a"]]
      #
      # Type:
      #   enumeratePaths :: [String] -> [[String]]
      enumeratePaths = attrPath: let
        proc = acc: attrName: let
          prev = builtins.head acc;
        in if ( builtins.length prev ) == 0 then [attrPath] else
           [( prev ++ [attrName] )] ++ acc;
      in builtins.foldl' proc [] sysRules.allowPackage;


    }; # End `lib'


    # ------------------------------------------------------------------------ #

  in {

    legacyPackages = let

      # Drop first two elements from each attribute path and handle those which
      # only apply to a single system.
      sysRules = let
        # Drop `legacyPackages' from the attribute paths.
        systems = builtins.mapAttrs ( _: map builtins.tail ) rules;
        collectForSystem = system: let
          proc = acc: attrPath:
            if builtins.elem ( builtins.head attrPath ) [system null]
            then acc ++ [( builtins.tail attrPath )]
            else acc;
        in builtins.foldl' proc [];
      in builtins.mapAttrs collectForSystem systems;

      # genLegacyPackages system
      # ------------------------
      # Generate a set of legacy packages for the given system.
      #
      # Example:
      #  genLegacyPackages "x86_64-linux"
      #  => { python3 = { pip = <derivation>; }; ... }
      #
      # Type:
      #   genLegacyPackages :: String -> AttrSet
      genLegacyPackages = system: let

        # Get a configured `nixpkgs' for the given system.
        base = import nixpkgs.outPath { inherit system config; };

        # Set `recurseForDerivations' to `true' for the given attribute paths.
        withAllowRecursive = let
          proc = pkgs: attrPath:
            lib.setAttrAt ( attrPath ++ ["recurseForDerivations"] ) true pkgs;
        in builtins.foldl proc base sysRules.allowRecursive;

        # Set `recurseForDerivations' to `false' for the given attribute paths.
        withDisallowRecursive = let
          proc = pkgs: attrPath: lib.removeAttrAt attrPath pkgs;
        in builtins.foldl' proc withAllowRecursive sysRules.allowRecursive;

        # Remove the given attribute paths.
        withDisallowPackage = let
          proc = pkgs: attrPath: lib.removeAttrAt attrPath pkgs;
        in builtins.foldl' proc withDisallowRecursive sysRules.disallowPackage;

        # Add back the given attribute paths and ensure all parent paths
        # allow `recurseForDerivations'.
        withAllowPackage = let
          proc = pkgs: attrPath: let
            # Get the value of the attribute at `attrPath' from the original
            # package set, and set it in the new package set.
            old     = lib.getAttrAt attrPath base;
            withOld = lib.setAttrAt attrPath old pkgs;
            # Ensure all parent paths allow `recurseForDerivations'.
            withParents = let
              parents = builtins.tail ( lib.enumeratePaths attrPath );
              proc    = attrs: parent: let
                path = parent ++ ["recurseForDerivations"];
              in lib.setAttrAt path true attrs;
            in builtins.foldl' proc withOld parents;
          in withParents;
        in builtins.foldl' proc withDisallowPackage sysRules.allowPackage;

      in withAllowPackage;

    in lib.eachDefaultSystemMap genLegacyPackages;


  };  # End `outputs'


  # -------------------------------------------------------------------------- #

}  # End flake


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
