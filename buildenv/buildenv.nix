{
  activationScripts_storePath ? "@activationScripts@",
  defaultEnvrc_storePath ? "@defaultEnvrc@",
  coreutils_storePath ? "@coreutils@",
  floxBuildenv_storePath ? "@out@",
  manifestLock,
  name ? "environment",
  serviceConfigYaml ? null,
}:
let

  # Ensure that `*_storePath` arguments are valid store paths
  # and declare a dependency on these paths.
  #
  # Note:
  # builtins.storePath :: string -> string
  # ensures that its argument is a valid store path
  # and returns a string with the path added to its string context[1].
  #
  # [1]: <https://nix.dev/manual/nix/2.24/language/string-context>
  activationScripts = builtins.storePath activationScripts_storePath;
  defaultEnvrc = builtins.storePath defaultEnvrc_storePath;
  floxBuildEnv = builtins.storePath floxBuildenv_storePath;
  coreutils = builtins.storePath coreutils_storePath;

  # A helpful library function copied from nixpkgs/lib/attrsets.nix.
  # foldlAttrs = f: init: set:
  #   builtins.foldl'
  #     (acc: name: f acc name set.${name})
  #     init
  #     (builtins.attrNames set);

  # The system we're building for.
  system = builtins.currentSystem;

  # Copy manifest file into the store for access within derivations.
  manifestLockFile = builtins.path { path = manifestLock; };

  # Parse the manifest file.
  manifestLockData = builtins.fromJSON (builtins.readFile manifestLock);
  manifestData = manifestLockData.manifest;

  build = if (builtins.hasAttr "build" manifestData) then manifestData.build else { };
  hook = if (builtins.hasAttr "hook" manifestData) then manifestData.hook else { };
  profile = if (builtins.hasAttr "profile" manifestData) then manifestData.profile else { };
  vars =
    if (builtins.hasAttr "vars" manifestData) then
      (builtins.toFile "envrc-vars" (
        builtins.concatStringsSep "" (
          builtins.map (n: "export ${n}=\"${builtins.getAttr n manifestData.vars}\"\n") (
            builtins.attrNames manifestData.vars
          )
        )
        # alternative ... worth it?
        #      foldlAttrs (
        #        acc: n: v: acc + "export ${n}=\"${v}\"\n"
        #      ) "" manifestData.vars
      ))
    else
      null;

  # Calculate environment outputs.
  environmentOutputs = [
    "runtime"
    "develop"
  ] ++ (builtins.map (x: "build-${x}") (builtins.attrNames build));

  createManifestChunks =
    [
      # static chunks
      ''
        export PATH="${coreutils}/bin''${PATH:+:}''${PATH}"
        mkdir -p $out/activate.d
        cp --no-preserve=mode ${manifestLockFile} $out/manifest.lock
        cp --no-preserve=mode ${defaultEnvrc} $out/activate.d/envrc
      ''
      # [vars] section
      (
        if vars == null then
          ""
        else
          ''
            cat ${vars} >> $out/activate.d/envrc
          ''
      )
      # [hook] section
      (
        if (builtins.hasAttr "on-activate" hook) then
          ''
            cp ${builtins.toFile "hook-on-activate" hook."on-activate"} $out/activate.d/hook-on-activate
          ''
        else
          ""
      )
      # service-config.yaml section
      (
        if (serviceConfigYaml == null) then
          ""
        else
          ''
            cp ${/. + serviceConfigYaml} $out/activate.d/service-config.yaml
          ''
      )
    ]
    ++ (
      # [profile] section
      builtins.map
        (
          i:
          if (builtins.hasAttr i profile) then
            let
              f = builtins.toFile "profile-${i}" (builtins.getAttr i profile);
            in
            "cp ${f} $out/activate.d/profile-${i}\n"
          else
            ""
        )
        [
          "bash"
          "fish"
          "tcsh"
          "zsh"
        ]
    )
    ++ (
      # [build] section
      builtins.map (
        i:
        let
          b = builtins.getAttr i build;
        in
        (
          if (builtins.hasAttr "command" b) then
            (
              let
                f = builtins.toFile "build-${i}" (builtins.getAttr "command" b);
              in
              ''
                mkdir -p $out/package-builds.d
                cp ${f} $out/package-builds.d/${i}
              ''
            )
          else
            ""
        )
      ) (builtins.attrNames build)
    );

  createManifestScript = builtins.toFile "create-manifest-script" (
    builtins.concatStringsSep "" createManifestChunks
  );

  # Create manifest package as derivation which invokes above script.
  manifestPackage = builtins.derivation {
    name = "manifest";
    inherit system;
    builder = "/bin/sh";
    args = [
      "-eux"
      createManifestScript
    ];
  };

  # Calculate inputSrcs by noting all storePaths for this system's
  # packages found in the packages list.
  inputSrcs = builtins.concatMap (
    package:
    if package.system == system then
      (
        if (builtins.hasAttr "outputs" package) then
          (
            # Important: report storePaths rather than strings because
            # the updated string context populates `inputSrcs` for the
            # resulting derivation.
            let
              outputsToInstall =
                if (builtins.hasAttr "outputs_to_install" package) then
                  (builtins.getAttr "outputs_to_install" package)
                else if (builtins.hasAttr "outputs-to-install" package) then
                  (
                    # XXX kebab-case flake lock bug
                    builtins.getAttr "outputs-to-install" package
                  )
                else
                  null;
              # Unfortunately, due to pkgdb limitations in the 1.0 release we
              # adopted the convention of installing all outputs for every
              # package, so for the _short_ term while we migrate to the new
              # buildenv we will continue this practice to keep the experience
              # consistent for users. However, it won't be long before we
              # switch over to the more correct strategy of honoring
              # outputs_to_install, and when we do we can remove "true ||"
              # from the following conditional.
              filteredOutputs =
                if (true || outputsToInstall == null) then
                  (builtins.attrValues package.outputs)
                else
                  (builtins.map (x: builtins.getAttr x package.outputs) outputsToInstall);
            in
            map (p: builtins.storePath p) filteredOutputs
          )
        else
          [ ]
      )
    else
      [ ]
  ) manifestLockData.packages;

in
# Throw a more meaningful error when lockfile version is < 1.
assert (builtins.hasAttr "lockfile-version" manifestLockData);
assert manifestLockData."lockfile-version" != "0";
builtins.derivation {
  inherit name;
  builder = "${floxBuildEnv}/lib/builder.pl";
  outputs = environmentOutputs;

  # Pull in external attributes and those calculated above.
  inherit
    activationScripts
    inputSrcs
    manifestPackage
    system
    ;

  # If the special attribute __structuredAttrs is set to true, the
  # other derivation attributes are serialised in JSON format and
  # made available to the builder via the file .attrs.json in the
  # builder’s temporary directory. This obviates the need for
  # passAsFile since JSON files have no size restrictions, unlike
  # process environments.
  # https://nix.dev/manual/nix/2.18/language/advanced-attributes#adv-attr-structuredAttrs
  __structuredAttrs = true;

  # This attribute allows builders access to the references graph of
  # their inputs. The attribute is a list of inputs in the Nix store
  # whose references graph the builder needs to know. The value of
  # this attribute should be a list of pairs `[ name1 path1 name2
  # path2 ...  ]`. The references graph of each *pathN* will be stored
  # in a text file *nameN* in the temporary build directory. The text
  # files have the format used by `nix-store --register-validity`
  # (with the deriver fields left empty). For example, when the
  # following derivation is built:
  #
  # ```nix
  # derivation {
  #   ...
  #   exportReferencesGraph = [ "libfoo-graph" libfoo ];
  # };
  # ```
  #
  # the references graph of `libfoo` is placed in the file
  # `libfoo-graph` in the temporary build directory.
  #
  # `exportReferencesGraph` is useful for builders that want to do
  # something with the closure of a store path. Examples include the
  # builders in NixOS that generate the initial ramdisk for booting
  # Linux (a `cpio` archive containing the closure of the boot script)
  # and the ISO-9660 image for the installation CD (which is populated
  # with a Nix store containing the closure of a bootable NixOS
  # configuration).
  #
  # https://nix.dev/manual/nix/2.18/language/advanced-attributes#adv-attr-exportReferencesGraph

  # N.B. with __structuredAttrs set this takes a slightly different (and
  # undocumented) form:
  #
  # derivation {
  #   ...
  #   exportReferencesGraph.<name> = [ path1 path2 ... ]
  # }
  #
  # ... and the effect of this is to create the following in .attrs.json:
  #
  # "exportReferencesGraph": {
  #   "<name>": [ path1 path2 ]
  # }
  # "<name>": [
  #   { derivationInfo for requisite1 },
  #   { derivationInfo for requisite2 },
  #   ...
  # ]
  #
  # The `builder.pl` script is responsible for parsing this when computing
  # the contents of requisites.txt for each output.
  exportReferencesGraph.graph = inputSrcs ++ [
    activationScripts
    manifestPackage
  ];
}
