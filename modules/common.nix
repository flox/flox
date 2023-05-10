{
  config,
  lib,
  system,
  context,
  namespace,
  ...
}: let
  # Assumption that flox-floxpkgs is a direct input
  floxpkgs = context.inputs.flox-floxpkgs;
  pkgs = context.nixpkgs;
in {
  options = with lib; {
    inline = mkOption {
      # TODO actual type
      type = types.unspecified;
      default = {};
      description = lib.mdDoc ''
        Escape hatch to inline Nix expressions for packages. The syntax is
        identical to what can be put in a toplevel flake.nix as an argument to
        `flox-floxpkgs.project`; see
        [this template](https://github.com/flox/floxpkgs-template/blob/main/flake.nix)
        for more details. In general, the top-level `packages` attribute should
        be used instead of `inline.packages` whenever possible.
      '';
      example = lib.literalExpression ''
        {
          packages = {
            myPython = {python3}:
              python3.withPackages (pythonPackages: with pythonPackages; [pandas]);
          };
        }
      '';
    };

    packages = mkOption {
      # TODO actual type
      type = types.attrsOf types.anything;
      default = {};
      description = lib.mdDoc ''
        Packages to include in the environment. A number of formats are
        supported:

        - `<channel>.<name>`

            - `<channel>` can be any channel subscribed to; run `flox channels` to
            list current subscriptions
            - The value of `<channel>.<name>` must be an attribute set with the
            following optional attributes:
                - `version` and `stability` strings. Available versions and stabilities
                can be found with `flox search -c <channel> <name>`.
                - `meta.priority`, which must be a number and defaults to `5`,
                allows resolving conflicts between two packages that provide the
                same file. For example, if two packages both provide a binary
                `foo` but one package has priority set to `4`, that package's
                version of `foo` will be present in the environment.
            - A complete example is:
            ```
            packages.nixpkgs-flox.hello = {
              stability = "unstable";
              version = "2.12.1";
              meta.priority = 4;
            };
            ```

        - `<self>.<name>`
            This installs a package defined in the same project as a flox
            environment, for example in `pkgs/my-pkg/default.nix`.

        - `<flake>.<name>`
            This supports installing packages from an arbitrary Nix flake. In
            general, installing from a channel is more performant, but this can
            be useful to use Nix software packaged in a flake that has not yet
            been packaged for flox.
      '';
      example = {
        nixpkgs-flox = {
          ripgrep = {
            version = "13.0.0";
            stability = "unstable";
          };
        };
        self.my-pkg = {};
        "github:vlinkz/nix-editor".nixeditor = {};
      };
    };

    catalogPath = mkOption {
      internal = true;
      type = types.nullOr types.path;
      default = null;
    };

    newCatalogPath = mkOption {
      internal = true;
      type = types.path;
    };

    manifestPath = mkOption {
      internal = true;
      type = types.path;
    };

    ###################
    # Copied from NixOS
    ###################
    system = {
      path = mkOption {
        internal = true;
        # description = lib.mdDoc ''
        #   The packages you want in the boot environment.
        # '';
      };
    };

    #######################
    # End copied from NixOS
    #######################

    packagesList = mkOption {
      internal = true;
      type = types.listOf types.package;
      default = [];
      # example = literalExpression "[ pkgs.firefox pkgs.thunderbird ]";
      # description = lib.mdDoc ''
      #   The set of packages that appear in
      #   /run/current-system/sw.  These packages are
      #   automatically available to all users, and are
      #   automatically updated every time you rebuild the system
      #   configuration.  (The latter is the main difference with
      #   installing them in the default profile,
      #   {file}`/nix/var/nix/profiles/default`.
      # '';
    };
  };

  config = let
    # helper
    notDerivation = x: ! (lib.isDerivation x);

    catalog =
      if config.catalogPath == null
      then {}
      else builtins.fromJSON (builtins.readFile config.catalogPath);

    # path getters
    # for channel and flake - a channel has an evalCatalog while a flake does not
    # for catalog and flake - the catalog path is the JSON path in catalog.json, and the flake path is the attribute path to the derivation
    getChannelCatalogPath = channelName: packageAttrPath: packageConfig:
      [
        channelName
        system
        packageConfig.stability or "stable"
      ]
      ++ packageAttrPath
      ++ [
        packageConfig.version or "latest"
      ];

    getChannelFlakePaths = packageAttrPath: packageConfig: let
      version =
        if packageConfig ? version
        then [
          (
            builtins.replaceStrings
            ["."]
            ["_"]
            packageConfig.version
          )
        ]
        else [];
    in [
      (
        [
          "evalCatalog"
          system
          packageConfig.stability or "stable"
        ]
        ++ packageAttrPath
        ++ version
      )
    ];

    # used for both flakes and self
    getFlakeCatalogPath = channelName: packageAttrPath: _:
      [
        channelName
        system
      ]
      ++ packageAttrPath;

    # used for both flakes and self
    getFlakeFlakePaths = packageAttrPath: _: [
      ([
          "packages"
          system
        ]
        ++ packageAttrPath)
      ([
          "legacyPackages"
          system
        ]
        ++ packageAttrPath)
    ];

    # utility function - should be in lib?
    # f is a function that takes a name and value and returns a string categorizing that name and
    # value
    groupAttrSetBy = f: attrSet: let
      listWithKeys =
        lib.mapAttrsToList (name: value: let
          groupByString = f name value;
        in {
          "${groupByString}" = {"${name}" = value;};
        })
        attrSet;
    in
      # we have [{groupByString = {name = value};}], and we know every name is unique, so combine
      # all attribute sets with the same groupByString
      builtins.zipAttrsWith (
        _: values:
          builtins.foldl'
          lib.recursiveUpdate
          {}
          values
      )
      listWithKeys;

    groupedChannels =
      groupAttrSetBy (
        channelName: _:
          if channelName == "self"
          then "flakes"
          else if lib.isStorePath channelName
          then "storePaths"
          else let
            fetchedChannel = builtins.getFlake channelName;
          in
            if builtins.hasAttr "evalCatalog" fetchedChannel
            then "channels"
            else "flakes"
      )
      config.packages;

    # partially apply generateFakeCatalog to the appropriate getters
    packagesWithDerivation =
      builtins.concatLists (lib.mapAttrsToList (getDerivationsForPackages getChannelCatalogPath getChannelFlakePaths) (groupedChannels.channels or {}))
      ++ builtins.concatLists (lib.mapAttrsToList (getDerivationsForPackages getFlakeCatalogPath getFlakeFlakePaths) (groupedChannels.flakes or {}));

    # Inline capacitated projects exposes capacitor interface
    inline =
      # Note, this does not re-expose current flake's self derivations, only re-uses its inputs
      let
        self =
          context.self
          // {
            # TODO: support sub-flakes, aka named environments
            # outPath = context.self.outPath + "/dir";

            # Fixed point operation that normally happens in call-flake.nix
            inputs = context.self.inputs // {inherit self;};
          }
          // project;
        project =
          context.inputs.flox-floxpkgs.inputs.capacitor.lib.capacitor.capacitate.capacitate
          {}
          self.inputs
          (
            if lib.isFunction (config.inline or null)
            then arg: (config.inline arg)
            else _: config.inline
          );
      in
        project;
    inlineCapacitorPackages =
      lib.mapAttrsToList (
        name: drv: let
          publishData =
            floxpkgs.lib.readPackage {
              attrPath = ["floxEnvs" system] ++ namespace ++ ["inline" "packages" system name];
              flakeRef = "self";
            } {analyzeOutput = true;}
            drv;
        in {
          inherit drv publishData;
        }
      )
      ## We only inject top-level packages
      inline.packages.${system} or {};

    storePaths = builtins.attrNames (groupedChannels.storePaths or {});

    getDerivationsForPackages = catalogPathGetter: flakePathsGetter: channelName: channelPackages: let
      # in order to support nested packages, we have to recurse until no attributes are attribute
      # sets, or there is a "config" attribute set
      isNotPackageConfig = attrs: ! attrs ? "meta" && builtins.any (value: builtins.isAttrs value) (builtins.attrValues attrs);
      # extract a list from the nested configuration format
      # return list of a packagesAttrSets where a packageAttrSet is of the form
      # {
      #   attrPath = ["python3Packages" "requests"];
      #   packageConfig = {
      #     meta = {};
      #     version = "1.12";
      #   };
      # };
      packageAttrSetsList = lib.collect (attrs: attrs ? attrPath && attrs ? packageConfig) (lib.mapAttrsRecursiveCond isNotPackageConfig (attrPath: value: {
          inherit attrPath;
          packageConfig = value;
        })
        channelPackages);

      # parition packages based on whether they are already in the catalog
      partitioned =
        builtins.partition (
          packageAttrSet:
          # Do not lock packages from self (eg: custom packages)
            channelName
            != "self"
            && lib.hasAttrByPath (catalogPathGetter channelName packageAttrSet.attrPath packageAttrSet.packageConfig) catalog
        )
        packageAttrSetsList;

      alreadyInCatalog =
        builtins.map (
          packageAttrSet: let
            catalogPath = catalogPathGetter channelName packageAttrSet.attrPath packageAttrSet.packageConfig;
          in rec {
            fakeDerivation = floxpkgs.lib.mkFakeDerivation (
              lib.recursiveUpdate (lib.getAttrFromPath catalogPath catalog) (
                if packageAttrSet ? packageConfig.meta
                then {eval.meta = packageAttrSet.packageConfig.meta;}
                else {}
              )
            );
            publishData = fakeDerivation.meta.publishData;
            # for informative error messages
            inherit channelName;
            # for informative error messages
            inherit (packageAttrSet) attrPath;
            inherit catalogPath;
          }
        )
        partitioned.right;
      fromChannel = let
        # todo let readPackage fetch the flake (technically right now there's a race condition)
        fetchedFlake =
          if channelName == "self"
          then context.self
          else builtins.getFlake channelName;
      in
        builtins.map (
          packageAttrSet: let
            catalogPath = catalogPathGetter channelName packageAttrSet.attrPath packageAttrSet.packageConfig;
            flakePaths = flakePathsGetter packageAttrSet.attrPath packageAttrSet.packageConfig;
            # find the first flake path that exists in fetchedFlake
            flakePath = let
              maybeFlakePath =
                builtins.foldl' (
                  foundFlakePath: flakePath:
                    if foundFlakePath != []
                    then foundFlakePath
                    else if lib.hasAttrByPath flakePath fetchedFlake
                    then flakePath
                    else []
                ) []
                flakePaths;
            in
              if maybeFlakePath != []
              then maybeFlakePath
              else let
                flakePathsToPrint = builtins.concatStringsSep " or " (builtins.map (flakePath: builtins.concatStringsSep "." flakePath) flakePaths);
              in
                throw "Channel ${channelName} does not contain ${flakePathsToPrint}";
            maybeFakeDerivation = lib.getAttrFromPath flakePath fetchedFlake;
            publishData =
              # if we have a fake derivation, add some additional meta required
              # by flox list to correctly display information about the catalog
              # this derivation came from (e.g. nixpkgs-flox) rather than the
              # original source it was built from (e.g. nixpkgs).
              # This is not reachable for self.
              if maybeFakeDerivation ? meta.publishData
              then let
                publish_element = let
                  flakeRef = channelName;
                in rec {
                  # TODO deduplicate with readPackage
                  originalUrl =
                    if flakeRef == "self"
                    then "." # TODO: use outPath?
                    else
                      (
                        if flakeRef == null || builtins.match ".*:.*" flakeRef == []
                        then flakeRef
                        else "flake:${flakeRef}"
                      );
                  # TODO deduplicate with readPackage and figure out a better
                  # way to store flake resolution information
                  url =
                    if flakeRef == "self"
                    then ""
                    else let
                      flake =
                        builtins.getFlake flakeRef;
                      # this assumes that either flakeRef is not indirect, or if
                      # it is indirect, the flake it resolves to contains a
                      # branch
                    in "${originalUrl}/${flake.rev}";
                  storePaths = maybeFakeDerivation.meta.publishData.element.storePaths;
                  attrPath = flakePath;
                };
              in
                maybeFakeDerivation.meta.publishData
                // {
                  inherit publish_element;
                }
              else
                floxpkgs.lib.readPackage {
                  # TODO use namespace and attrPath instead of passing entire flakePath as attrPath
                  attrPath = flakePath;
                  flakeRef = channelName;
                } {analyzeOutput = true;}
                maybeFakeDerivation;

            # The floxEnv must be identical for the locking and locked build, so we have to
            # - call mkFakeDerivation even if we already have a fake derivation, because the version
            #   of mkFakeDerivation used by the catalog plugin may be different than the version
            #   called in this file
            # - wrap derivations from flakes in a fake derivation, because that's what
            #   will happen once they are put in the catalog
            fakeDerivation = floxpkgs.lib.mkFakeDerivation (
              lib.recursiveUpdate publishData
              (
                if packageAttrSet ? packageConfig.meta
                then {eval.meta = packageAttrSet.packageConfig.meta;}
                else {}
              )
            );
          in
            # this function returns just the entries for this channel, and the caller adds channelName to the complete catalog
            rec {
              # publishData has publish_element, which
              # fakeDerivation.meta.publishData does not
              inherit fakeDerivation publishData;
              # for informative error messages
              inherit channelName;
              # for informative error messages
              inherit (packageAttrSet) attrPath;
              inherit catalogPath;
            }
            // lib.optionalAttrs (channelName == "self")
            {drv = maybeFakeDerivation;}
        )
        partitioned.wrong;
    in
      alreadyInCatalog ++ fromChannel;

    # we could check uniqueness in O(n log n) by first sorting all elements by storePaths, but I don't think that's
    # worth my time at the moment
    # instead, for every package, compare to every package and every store path
    uniquePackagesWithDerivation =
      builtins.map (
        packageWithDerivation1:
        # we need to throw if packageWithDerivation1 is not unique, but if it is unique, we don't need the
        # result of this computation, so use deepSeq
        # compare against all packages from flakes and channels
        # TODO use genericClosure instead?
          builtins.deepSeq (builtins.map (
              packageWithDerivation2:
                if packageWithDerivation1.catalogPath == packageWithDerivation2.catalogPath
                then null
                else if
                  (builtins.sort builtins.lessThan packageWithDerivation1.publishData.element.storePaths)
                  == (builtins.sort builtins.lessThan packageWithDerivation2.publishData.element.storePaths)
                then throw "package ${builtins.concatStringsSep "." packageWithDerivation1.catalogPath} is identical to package ${builtins.concatStringsSep "." packageWithDerivation2.catalogPath}"
                else null
            )
            packagesWithDerivation)
          # compare against storePaths
          (
            builtins.deepSeq
            (builtins.map (
                storePath:
                  if packageWithDerivation1.publishData.element.storePaths == [storePath]
                  then throw "package ${builtins.concatStringsSep "." packageWithDerivation1.catalogPath} is identical to store path ${storePath}"
                  else null
              )
              storePaths)
            # pass packageWithDerivation1 through the map - in other words, do nothing
            packageWithDerivation1
          )
      )
      packagesWithDerivation;

    sortedPackagesWithDerivation = builtins.sort (packageWithDerivation1: packageWithDerivation2: packageWithDerivation1.catalogPath < packageWithDerivation2.catalogPath) uniquePackagesWithDerivation;

    # since storePaths are specified in the attrPath, we don't need to check for uniqueness
    sortedStorePaths = builtins.sort (storePath1: storePath2: storePath1 < storePath2) storePaths;

    # extract a list of derivations
    packagesList =
      builtins.map (packageWithDerivation: packageWithDerivation.drv or packageWithDerivation.fakeDerivation)
      sortedPackagesWithDerivation
      # types.package calls builtins.storePath
      ++ sortedStorePaths
      # TODO check for duplication
      ++ builtins.map (inlineCapacitorPackage: inlineCapacitorPackage.drv) inlineCapacitorPackages;

    # store paths are not added to the catalog
    newCatalog =
      builtins.foldl' lib.recursiveUpdate {}
      (
        (builtins.map
          (packageWithDerivation:
            lib.setAttrByPath
            packageWithDerivation.catalogPath
            packageWithDerivation.publishData)
          sortedPackagesWithDerivation)
        ++ (builtins.map
          (
            inlineCapacitorPackage:
              lib.setAttrByPath
              # this is kind of meaningless
              inlineCapacitorPackage.publishData.element.attrPath
              inlineCapacitorPackage.publishData
          )
          inlineCapacitorPackages)
      );

    # For flake:
    # {
    #   "active": true,
    #   "attrPath": "legacyPackages.aarch64-darwin.hello",
    #   "originalUrl": "flake:nixpkgs",
    #   "outputs": null,
    #   "priority": 5,
    #   "storePaths": [
    #     "/nix/store/gq5b6y0zxvpfxywi600ahlcg3mnscv93-hello-2.12.1"
    #   ],
    #   "url": "github:flox/nixpkgs/2788904d26dda6cfa1921c5abb7a2466ffe3cb8c"
    # }

    # For channel:
    # {
    #   "active": true,
    #   "attrPath": "evalCatalog.aarch64-darwin.stable.hello",
    #   "originalUrl": "flake:nixpkgs-flox",
    #   "outputs": null,
    #   "priority": 5,
    #   "storePaths": [
    #     "/nix/store/lc9cci22gfxd7xaqjdvz3kkd09g4g0g7-hello-2.12.1"
    #   ],
    #   "url": "github:flox/nixpkgs-flox/5cdedb611cf745c734b0268346e940a8b1e33b45"
    # },
    packageManifestElements =
      builtins.map (
        packageWithDerivation: let
          element =
            # if this is a publish of a publish, use it
            packageWithDerivation.publishData.publish_element
            or packageWithDerivation.publishData.element;
        in {
          active = true;
          inherit (element) url originalUrl storePaths;
          attrPath = builtins.concatStringsSep "." element.attrPath;
          outputs = element.outputs or null;
        }
      )
      # manifest.json needs to have a non-random ordering for flox list
      sortedPackagesWithDerivation;
    storePathManifestElements =
      builtins.map (storePath: {
        active = true;
        storePaths = [
          storePath
        ];
      })
      sortedStorePaths;

    inlinePackagesManifestElements =
      builtins.map (inlineCapacitorPackage: let
        element = inlineCapacitorPackage.publishData.element;
      in {
        active = true;
        inherit (element) url originalUrl storePaths;
        # this is kind of meaningless
        attrPath = builtins.concatStringsSep "." element.attrPath;
        outputs = element.outputs or null;
      })
      inlineCapacitorPackages;

    manifestJSON = builtins.toJSON {
      version = 2;
      elements = packageManifestElements ++ storePathManifestElements ++ inlinePackagesManifestElements;
    };
  in {
    manifestPath = builtins.toFile "profile" manifestJSON;
    inherit packagesList;
    newCatalogPath = pkgs.writeTextFile {
      name = "catalog";
      destination = "/catalog.json";
      text = builtins.unsafeDiscardStringContext (builtins.toJSON newCatalog);
    };

    passthru.inline = inline;
    passthru.catalog = newCatalog;
    # packages installed by store path,
    # for which no further information can be derived
    passthru.installedStorePaths = sortedStorePaths;
  };
  imports = [
    (lib.mkAliasOptionModule ["integrations"] ["inline"])
  ];
}
