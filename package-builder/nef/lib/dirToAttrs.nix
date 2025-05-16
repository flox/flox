{ lib }:

{
  /**
      This function takes a directory structure of nix expressions
      and turns them ino a recursive attrset.

      The name of entries is eitehr the base name of `.nix` files found
      or the directory name iff `<parent>/<name>/default.nix` exists.

      # Example

      ```nix
      dirToAttrs ../tests/pkgs
      =>
      {
          type = "directory";
          path = "...";
          entries = {
              baz = { type = "nix"; path = "..." }
              foo = {
                  type = "directory";
                  path = "...";
                  entries = {
                      bar = { type = "nix"; path = "..." };
                  };
              }
          }
      }
      ```

      # Type

      ```
      dirToAddrs :: Path | String -> Root

      where

      Root :: Directory
      Directory :: {
        path = String
        type = "directory"
        entries = { name -> Nix | Directopry }
      }
      Nix :: {
        path = String
        type = "nix"
      }
      ```

      # Arguments

      `dir`
      : The directory to import
  */
  dirToAttrs =
    dir:
    let
      pathToEntries =
        fileOrDir:

        let
          exists = builtins.pathExists fileOrDir;
          filetype = builtins.readFileType fileOrDir;

          directoryWithDefault = {
            type = "nix";
            path = "${fileOrDir}/default.nix";
          };

          nixPackageFile = {
            type = "nix";
            path = fileOrDir;
          };

          directoryAsSubset =
            let
              entries = lib.attrValues (
                lib.mapAttrs (
                  name: _: lib.nameValuePair (lib.removeSuffix ".nix" name) (pathToEntries "${fileOrDir}/${name}")
                ) (builtins.readDir fileOrDir)
              );
              validEntries = lib.filter (v: (v ? value && v.value != null && v.value != { })) entries;

              # Regular files should be preferred over directories, so that e.g.
              # foo.nix can be used to declare a further import of the foo directory
              entryAttrs = lib.listToAttrs (lib.sort (a: b: a.value.type == "regular") validEntries);

            in
            if builtins.length validEntries > 0 then
              {
                type = "directory";
                path = fileOrDir;
                entries = entryAttrs;
              }
            else
              null;

          entry =
            if filetype == "directory" && builtins.pathExists "${fileOrDir}/default.nix" then
              directoryWithDefault
            else if filetype == "directory" then
              directoryAsSubset
            else if
              filetype == "regular" && lib.hasSuffix ".nix" fileOrDir && !lib.hasSuffix "flake.nix" fileOrDir
            then
              nixPackageFile
            else
              null;

        in
        if exists then
          entry
        else
          builtins.trace "Not importing any attributes because the directory ${dir} doesn't exist" null;

      result = pathToEntries dir;
    in
    if result == null then
      {
        type = "directory";
        path = dir;
        entries = { };
      }
    else
      result;
}
