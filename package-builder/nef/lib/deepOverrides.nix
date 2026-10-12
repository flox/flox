{ lib }:
let
  # The marker that promotes a package directory to a deep override,
  # the same convention flox-lib's own channel packages use
  # (`deep = builtins.pathExists (path + "/deep-override")`,
  # `channel/utils/dirToAttrs.nix:22` in flox/flox-lib). A package
  # declared as a single `.nix` file has no directory of its own to
  # hold the marker, so it can never be a deep override.
  deepOverrideMarkerFile = "deep-override";

  isMarkedAsDeepOverride =
    node:
    node.type == "nix"
    && lib.hasSuffix "/default.nix" node.path
    && builtins.pathExists ("${lib.removeSuffix "/default.nix" node.path}/${deepOverrideMarkerFile}");

  # Keep the "nix" nodes of a `dirToAttrs` tree that satisfy `keep`, at
  # every nesting level. Directories are always descended into and are
  # dropped once nothing in them is kept.
  filterPkgsTree =
    keep: tree:
    tree
    // {
      entries = lib.filterAttrs (_: node: node.type == "nix" || node.entries != { }) (
        builtins.mapAttrs (_: node: if node.type == "directory" then filterPkgsTree keep node else node) (
          lib.filterAttrs (_: node: node.type == "directory" || keep node) tree.entries
        )
      );
    };

  # Split a `pkgs/` tree into its deep overrides and its shallow
  # packages. A marked package is left out of the shallow half, so it is
  # applied once, in the base, and its siblings resolve it from there.
  #
  # `instantiateFromSourceInfo` (`instantiate.nix`) uses the shallow half
  # as a repository's own package tree. `collectDeepOverrides` uses every
  # source's shallow half for the sibling stubs, and the consuming
  # project's deep half to find its overrides, since it has no lock
  # entry.
  partitionPkgsTree = tree: {
    deepTree = filterPkgsTree isMarkedAsDeepOverride tree;
    shallowTree = filterPkgsTree (node: !isMarkedAsDeepOverride node) tree;
  };

  # Every (attrPath, source) pair a locked package node's own
  # `deep_overrides` records, i.e. the lock's own statement of what it
  # overrides -- not a scan of anything fetched. Each entry is already
  # an attribute path (a list of component names), the same shape
  # `flattenOverrideAttrPaths` below produces from a directory scan, so
  # no join or split sits between the wire and this list. Mirrors the
  # "package" / "package_set" recursion `instantiate.nix`'s
  # `fetchFloxHubCatalog` uses to walk the same tree, but collects
  # pairs instead of instantiating, and only where a node actually
  # names an override, so a project with no deep overrides in its
  # closure fetches nothing extra.
  collectDeepOverrideSources =
    node:
    {
      "package" = map (attrPath: {
        inherit attrPath;
        inherit (node) source;
      }) (node.deep_overrides or [ ]);
      "package_set" = lib.concatMap collectDeepOverrideSources (lib.attrValues node.entries);
    }
    .${node.type};

  # Every (attrPath, source) pair reachable from a catalog closure's
  # lock, one catalog at a time. `nix` catalogs throw here exactly as
  # `instantiateCatalog` does, since they would fail on the same
  # closure moments later.
  collectClosureDeepOverridePairs =
    catalogSpecClosure:
    lib.concatMap (
      catalogSpec:
      {
        "nix" = throw "source inputs not currently supported";
        "floxhub" = collectDeepOverrideSources catalogSpec.packages;
      }
      .${catalogSpec.type}
    ) (lib.attrValues catalogSpecClosure);

  # Every attrPath a `partitionPkgsTree` override half marks, flattened
  # out of its physical directory shape into the same `{ attrPath; }`
  # list shape the lock's own pairs already come in. Only the
  # consuming project needs this: it has no lock entry to name its
  # overrides for it the way a dependency's does.
  flattenOverrideAttrPaths =
    prefix: tree:
    lib.concatMap (
      name:
      let
        node = tree.entries.${name};
        path = prefix ++ [ name ];
      in
      if node.type == "nix" then [ path ] else flattenOverrideAttrPaths path node
    ) (lib.attrNames tree.entries);

  # Keep one pair per source label and attribute path. Every package
  # locked from a repository lists that repository's whole set of deep
  # overrides, so two packages from one source yield identical pairs.
  # Kept as two, they would collide with each other in
  # `buildOverrideTree`.
  dedupeBySourceAndPath =
    pairs:
    lib.concatMap builtins.attrValues (
      builtins.attrValues (
        builtins.mapAttrs (
          _: sourcePairs:
          builtins.listToAttrs (map (p: lib.nameValuePair (lib.showAttrPath p.attrPath) p) sourcePairs)
        ) (lib.groupBy (p: p.sourceLabel) pairs)
      )
    );

  # Merge every source's (attrPath, sourceLabel, sourceInfo) triples into
  # one tree of the same shape `mkDeepOverrideOverlay` below walks: an
  # "override" node per override, carrying the source label and fetched
  # source it came from, and a "directory" per nesting level in between.
  #
  # Two sources contributing the same attribute path is an evaluation
  # error naming both; two sources contributing different attributes
  # under the same subdirectory merge, since neither actually
  # collides. The error lives in the attribute's own value, which
  # `builtins.mapAttrs` leaves as an unforced thunk, so it fires only
  # once that attribute is actually reached -- a consumer shadowing
  # the same name in its own `pkgs/` never reaches it (see
  # `mkDeepOverrideOverlay`'s self-binding).
  buildOverrideTree =
    attrPathPrefix: overrides:
    let
      grouped = lib.groupBy (o: builtins.head o.attrPath) overrides;
    in
    builtins.mapAttrs (
      name: group:
      let
        attrPathStr = lib.showAttrPath (attrPathPrefix ++ [ name ]);
        rest = map (o: o // { attrPath = builtins.tail o.attrPath; }) group;
        overridesHere = builtins.filter (o: o.attrPath == [ ]) rest;
        nested = builtins.filter (o: o.attrPath != [ ]) rest;
      in
      if nested != [ ] && overridesHere != [ ] then
        throw ''
          Deep override collision on '${attrPathStr}': defined by both
          '${(builtins.elemAt overridesHere 0).sourceLabel}' and '${(builtins.elemAt nested 0).sourceLabel}'.
        ''
      else if nested != [ ] then
        {
          type = "directory";
          entries = buildOverrideTree (attrPathPrefix ++ [ name ]) nested;
        }
      else if builtins.length overridesHere == 1 then
        {
          type = "override";
          inherit (builtins.head overridesHere) sourceLabel sourceInfo;
        }
      else
        throw ''
          Deep override collision on '${attrPathStr}': defined by both
          '${(builtins.elemAt overridesHere 0).sourceLabel}' and '${(builtins.elemAt overridesHere 1).sourceLabel}'.
        ''
    ) grouped;

  # Bound per override, so only an override that asks for `catalogs`
  # fails.
  catalogsDeniedError = throw ''
    A deep override cannot use catalog packages: it is folded into
    the base nixpkgs before any catalog is instantiated, so no
    catalog exists yet when it runs.
  '';

  # Build an overlay for deep overrides: `final: prev: resolved`.
  #
  # An override takes its inputs from `final`, like a shallow package
  # in `lib.nef.mkOverlay`. `final` is the namespace the base is later
  # extended into: the consuming project's `pkgs/` or a catalog
  # source's. A shallow package in that namespace reaches a deep
  # override evaluated there, as it reaches every other package.
  #
  # Two inputs are withheld. An override's own repository's shallow
  # packages are stubbed, because they exist only in that repository's
  # namespace. `catalogs` is denied, because no catalog exists when the
  # base is built.
  #
  # Known gap, untested: if a source's shallow package X also exists
  # upstream and that source's override reads X, a namespace that
  # shadows X does not reach the override. The stub returns the base X
  # and warns.
  #
  # The directory recursion duplicates `lib.nef.mkOverlay` so the stubs
  # and the `catalogs` denial stay out of the shallow package path.
  # Both share `lib.nef.callPackageIn` and `lib.nef.applyOverlay`.
  mkDeepOverrideOverlay =
    attrPath: currentScope: extensions: siblingTreesBySourceLabel:
    (
      final: prev:
      let
        # The entries of `sourceLabel`'s own shallow `pkgs/` tree at this
        # nesting level (`attrPath`), or none if that source has
        # nothing at this position. Siblings are scoped to one
        # override's own source, never merged across sources: an
        # override from repo R must not be warned or thrown about a
        # name that only exists as a sibling in repo S.
        siblingsFor =
          sourceLabel:
          (lib.foldl' (
            node: component:
            if builtins.hasAttr component node.entries then
              node.entries.${component}
            else
              {
                type = "directory";
                path = null;
                entries = { };
              }
          ) (siblingTreesBySourceLabel.${sourceLabel} or { entries = { }; }) attrPath).entries;

        # A warn-or-throw value for every name in a source's own
        # sibling entries at this level, checked against `prev` -- a
        # shallow package coinciding with a real upstream name warns
        # and falls back to it; one that doesn't, throws, naming the
        # source.
        mkStubs =
          sourceLabel: siblingEntries:
          builtins.mapAttrs (
            name: _node:
            let
              siblingAttrPathStr = lib.showAttrPath (attrPath ++ [ name ]);
            in
            if builtins.hasAttr name prev then
              lib.warn ''
                Deep override scope: '${siblingAttrPathStr}' is a shallow package in '${sourceLabel}', which also defines a deep override, but is not itself marked as one; using the '${siblingAttrPathStr}' already in scope instead of the unmarked definition.
              '' prev.${name}
            else
              throw ''
                Deep override scope: '${siblingAttrPathStr}' is referenced by a deep override, but is itself only a shallow package in '${sourceLabel}', not a deep override. Either mark it by adding a 'deep-override' file next to its 'default.nix' in '${sourceLabel}', or stop referencing it from the override.
              ''
          ) siblingEntries;

        resolved = builtins.mapAttrs (
          name: value:
          let
            attrPath' = attrPath ++ [ name ];
            attrPathStr = lib.showAttrPath attrPath';
          in
          {
            "override" =
              if !(builtins.hasAttr name prev) then
                throw ''
                  Deep override scope: '${attrPathStr}' is locked as a deep override by '${value.sourceLabel}', but '${attrPathStr}' does not exist in the base. Only packages that already exist in the base can be deeply overridden -- a deep override replaces an existing package, it cannot introduce a new one.
                ''
              else
                let
                  exprPath = "${value.sourceInfo.outPath}/${value.sourceInfo.dir or ""}/pkgs/${lib.concatStringsSep "/" attrPath'}/default.nix";

                  recursionGuardError = throw ''
                    Circular dependency detected.
                    The package '${attrPathStr}' defined in ${exprPath} directly or transitively imports itself.
                    For example by requesting a dependency '${name}'.
                    An expression can only access its own attribute path to override its existing value.
                  '';

                  # A stubbed name that is also deep-overridden at this
                  # level resolves to the override through `final`.
                  #
                  # `catalogs` and the self-reference go in selfBinding,
                  # the only `callPackageIn` argument that wins in every
                  # branch. The fallback branch ranks `final` above
                  # `currentScope`, and a namespace's `final` has a real
                  # `catalogs`.
                  selfBinding =
                    removeAttrs (mkStubs value.sourceLabel (siblingsFor value.sourceLabel)) (
                      builtins.attrNames extensions.entries
                    )
                    // {
                      catalogs = catalogsDeniedError;
                      ${name} = prev.${name} or recursionGuardError;
                    };

                  callPackage = lib.nef.callPackageIn final currentScope selfBinding;

                  errorContext = "while replacing '${attrPathStr}' by evaluating '${exprPath}'";
                in
                builtins.addErrorContext errorContext (callPackage exprPath { });

            "directory" =
              if !(builtins.hasAttr name prev) then
                throw ''
                  Deep override scope: '${attrPathStr}' is referenced as a deep-override package set, but '${attrPathStr}' does not exist in the base. Only packages that already exist in the base can be deeply overridden -- a deep override replaces an existing package, it cannot introduce a new one.
                ''
              else
                let
                  attrSet = prev.${name};
                  nestedOverlay = mkDeepOverrideOverlay attrPath' (
                    currentScope // final
                  ) value siblingTreesBySourceLabel;
                in
                builtins.addErrorContext "while extending package set '${attrPathStr}'" (
                  lib.nef.applyOverlay attrSet nestedOverlay attrPath'
                );
          }
          .${value.type}
        ) extensions.entries;
      in
      resolved
    );
in
{
  inherit partitionPkgsTree;

  /**
    Build one override tree out of every deep override reachable from
    a catalog closure's lock, plus the consuming project's own. The tree
    has the shape `mkDeepOverrideOverlay` walks: an "override" node per
    override, carrying the source label and fetched source it came from,
    and a "directory" per nesting level in between. Also returns each
    source's shallow `pkgs/` tree, keyed by source label, for the sibling
    stubs in `applyDeepOverrides`.

    Which attrPaths are overrides comes from the lock itself (each
    locked package node's `deep_overrides`), not from scanning a
    fetched source: a source is fetched to evaluate its override
    expressions and to find its own unmarked siblings, never to
    rediscover what it overrides. The one exception is the consuming
    project's own `pkgs/`, which has no lock entry to name its
    overrides for it, so those are still found by the `deep-override`
    marker (`partitionPkgsTree`'s override half).

    A locked attrPath the fetched source does not actually define is
    an evaluation error naming the source and the path, not a silent
    skip: the lock and the source it names are expected to agree.

    Packages locked from the same source share one override per
    attribute path. Two different sources overriding the same attribute
    path is an evaluation error naming both (see `buildOverrideTree`).

    # Arguments

    `catalogSpecClosure`
    : the locked catalog closure, as provided in a catalog lock file

    `sourceInfo`
    : the consuming project's own fetched source
  */
  collectDeepOverrides =
    { catalogSpecClosure, sourceInfo }:
    let
      lockedPairs = collectClosureDeepOverridePairs catalogSpecClosure;

      # One fetch per distinct locked source, keyed by its source label and
      # reused across every attrPath that source's lock entries name,
      # exactly as many fetches as the old scan-based discovery made.
      sourcesBySourceLabel = builtins.listToAttrs (
        map (source: lib.nameValuePair (lib.nef.instantiate.labelSource source) source) (
          lib.unique (map (p: p.source) lockedPairs)
        )
      );
      sourceInfoBySourceLabel =
        builtins.mapAttrs (_: lib.nef.instantiate.fetchSource) sourcesBySourceLabel
        // {
          "the consuming project" = sourceInfo;
        };

      # Every locked pair, labeled and checked against the source it
      # names. `builtins.head`/`builtins.tail` on `attrPath`, which
      # `buildOverrideTree` needs at every nesting level to group
      # these by key, forces this check for every pair regardless of
      # where in the tree it ends up -- no separate pass is needed to
      # surface a mismatch.
      checkedLockedPairs = map (
        pair:
        let
          sourceLabel = lib.nef.instantiate.labelSource pair.source;
          source = sourceInfoBySourceLabel.${sourceLabel};
          exprPath = "${source.outPath}/${source.dir or ""}/pkgs/${lib.concatStringsSep "/" pair.attrPath}/default.nix";
        in
        if builtins.pathExists exprPath then
          {
            inherit sourceLabel;
            inherit (pair) attrPath;
            sourceInfo = source;
          }
        else
          throw ''
            Deep override lock disagreement: '${sourceLabel}' locks '${lib.showAttrPath pair.attrPath}'
            as a deep override, but the fetched source has no '${exprPath}'.
          ''
      ) lockedPairs;

      # Every source's own shallow (unmarked) `pkgs/` tree, for the
      # per-source stub layers -- every labeled source needs this, not
      # only the ones with their own lock entry.
      partitionedBySourceLabel = builtins.mapAttrs (
        _: source: partitionPkgsTree (lib.nef.dirToAttrs "${source.outPath}/${source.dir or ""}/pkgs")
      ) sourceInfoBySourceLabel;

      ownOverridePairs = map (attrPath: {
        inherit attrPath;
        sourceLabel = "the consuming project";
        sourceInfo = sourceInfoBySourceLabel."the consuming project";
      }) (flattenOverrideAttrPaths [ ] partitionedBySourceLabel."the consuming project".deepTree);
    in
    {
      overrideTree = {
        type = "directory";
        entries = buildOverrideTree [ ] (dedupeBySourceAndPath (checkedLockedPairs ++ ownOverridePairs));
      };
      siblingTreesBySourceLabel = builtins.mapAttrs (
        _: parts: parts.shallowTree
      ) partitionedBySourceLabel;
    };

  /**
    Apply the override tree and per-source sibling trees assembled by
    `collectDeepOverrides` to `nixpkgs`, as a single overlay
    (`mkDeepOverrideOverlay` above) applied before any catalog or
    project is instantiated.

    A deep override can only replace an attribute `nixpkgs` already
    has. Overriding a missing package or package set throws when that
    attribute is accessed. flox-lib enforces the same rule
    (`deepAuthorityError` in flox/flox-lib `channel/root.nix`), so a
    dependency cannot add to the base without the root's consent.

    # Arguments

    `nixpkgs`
    : the base nixpkgs instance deep overrides are applied to

    `deepOverrides`
    : the `{ overrideTree; siblingTreesBySourceLabel; }` returned by
      `collectDeepOverrides`
  */
  applyDeepOverrides =
    nixpkgs: deepOverrides:
    let
      inherit (deepOverrides) overrideTree siblingTreesBySourceLabel;

      overlay = mkDeepOverrideOverlay [ ] {
        catalogs = catalogsDeniedError;
      } overrideTree siblingTreesBySourceLabel;
    in
    lib.nef.applyOverlay nixpkgs overlay [ ];
}
