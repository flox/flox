{
  interpreter_out_storePath ? "@interpreter_out@",
  flox_activations_out_storePath ? "@flox_activations_out@",
  interpreter_wrapper_storePath ? "@interpreter_wrapper@",
  defaultEnvrc_storePath ? "@defaultEnvrc@",
  coreutils_storePath ? "@coreutils@",
  floxBuildenv_storePath ? "@out@",
  manifestLock,
  name ? "environment",
  serviceConfigYaml ? null,
}:
let
  outdentScript = (import ./buildenvLib/default.nix).outdentText;
  # Ensure that `*_storePath` arguments are valid store paths
  # and declare a dependency on these paths.
  #
  # Note:
  # builtins.storePath :: string -> string
  # ensures that its argument is a valid store path
  # and returns a string with the path added to its string context[1].
  #
  # [1]: <https://nix.dev/manual/nix/2.24/language/string-context>
  interpreter_out = builtins.storePath interpreter_out_storePath;
  flox_activations_out = builtins.storePath flox_activations_out_storePath;
  interpreter_wrapper = builtins.storePath interpreter_wrapper_storePath;
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
  lockfilePath = builtins.path {
    path = manifestLock;
    name = "manifest.lock";
  };

  # Parse the manifest file.
  lockfile = builtins.fromJSON (builtins.readFile manifestLock);
  manifest = lockfile.manifest;

  buildSection = if (builtins.hasAttr "build" manifest) then manifest.build else { };
  hookSection = if (builtins.hasAttr "hook" manifest) then manifest.hook else { };
  profileSection = if (builtins.hasAttr "profile" manifest) then manifest.profile else { };
  vars =
    if (builtins.hasAttr "vars" manifest) then
      (builtins.toFile "envrc-vars" (
        builtins.concatStringsSep "" (
          builtins.map (n: "export ${n}=\"${builtins.getAttr n manifest.vars}\"\n") (
            builtins.attrNames manifest.vars
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
  ]
  ++ (builtins.map (buildId: "build-${buildId}") (builtins.attrNames buildSection));

  createRenderedEnvironmentChunks = [
    # static chunks
    ''
      export PATH="${coreutils}/bin''${PATH:+:}''${PATH}"
      "${coreutils}/bin/mkdir" -p $out/activate.d
      "${coreutils}/bin/cp" --no-preserve=mode ${lockfilePath} $out/manifest.lock
      "${coreutils}/bin/cp" --no-preserve=mode ${defaultEnvrc} $out/activate.d/envrc
    ''
    # [vars] section
    (
      if vars == null then
        ""
      else
        ''
          "${coreutils}/bin/cat" ${vars} >> $out/activate.d/envrc
        ''
    )
    # [hook] section
    (
      if
        (builtins.hasAttr "on-activate" hookSection && (builtins.getAttr "on-activate" hookSection) != null)
      then
        let
          contents = outdentScript (builtins.getAttr "on-activate" hookSection);
          scriptFile = builtins.toFile "hook-on-activate" contents;
        in
        ''
          "${coreutils}/bin/cp" ${scriptFile} $out/activate.d/hook-on-activate
        ''
      else
        ""
    )
    # service-config.yaml section
    (
      if (serviceConfigYaml == null) then
        ""
      else
        let
          serviceConfigYamlStorePath = builtins.path {
            path = serviceConfigYaml;
            name = "service-config.yaml";
          };
        in
        ''
          "${coreutils}/bin/cp" ${serviceConfigYamlStorePath} $out/service-config.yaml
        ''
    )
  ]
  ++ (
    # [profile] section
    builtins.map
      (
        shellType:
        if
          (builtins.hasAttr shellType profileSection && (builtins.getAttr shellType profileSection) != null)
        then
          let
            contents = outdentScript (builtins.getAttr shellType profileSection);
            scriptFile = builtins.toFile "profile-${shellType}" contents;
          in
          ''
            "${coreutils}/bin/cp" ${scriptFile} $out/activate.d/profile-${shellType}
          ''
        else
          ""
      )
      [
        "bash"
        "common"
        "fish"
        "tcsh"
        "zsh"
      ]
  )
  ++ (
    # [build] section
    builtins.map (
      buildId:
      let
        build = builtins.getAttr buildId buildSection;
      in
      (
        if (builtins.hasAttr "command" build && (builtins.getAttr "command" build) != null) then
          let
            contents = outdentScript (builtins.getAttr "command" build);
            scriptFile = builtins.toFile "build-${buildId}" contents;
          in
          ''
            "${coreutils}/bin/mkdir" -p $out/package-builds.d
            "${coreutils}/bin/cp" ${scriptFile} $out/package-builds.d/${buildId}
          ''
        else
          ""
      )
    ) (builtins.attrNames buildSection)
  );

  renderEnvironmentScript = builtins.toFile "render-environment-script" (
    builtins.concatStringsSep "" createRenderedEnvironmentChunks
  );

  # Create manifest package as derivation which invokes above script.
  manifestPackage = builtins.derivation {
    name = "manifest";
    inherit system;
    builder = "/bin/sh";
    args = [
      "-eux"
      renderEnvironmentScript
    ];
  };

  # Calculate inputSrcs by noting all storePaths for this system's
  # packages found in the packages list.
  inputSrcs = builtins.concatMap (
    package:
    if package.system == system then
      (
        if builtins.hasAttr "store_path" package then
          let
            storePath = builtins.getAttr "store_path" package;
            registeredStorePath = builtins.storePath storePath;
          in
          [ registeredStorePath ]
        else if (builtins.hasAttr "outputs" package) then
          (
            # Important: report storePaths rather than strings because
            # the updated string context populates `inputSrcs` for the
            # resulting derivation.
            let
              descriptor =
                if (builtins.hasAttr package.install_id manifest.install) then
                  (builtins.getAttr package.install_id manifest.install)
                else
                  throw "manifest does not contain a package with install ID '${package.install_id}'";
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
              getValidAttrs = (
                selected: pkg:
                builtins.map (
                  output:
                  if (builtins.elem output (builtins.attrNames pkg.outputs)) then
                    output
                  else
                    throw "${pkg.attr_path} has no output named '${output}'"
                ) selected
              );
              getV1Outputs = (
                pkg:
                # Filter out outputs named `stubs` because they're needed at build time,
                # but break things at run time. This may be unnecessary once we do
                # "outputs to install". The `stubs` outputs became a problem when adding
                # CUDA support.
                (builtins.attrNames (builtins.removeAttrs package.outputs [ "stubs" ]))
              );
              getV2Outputs = (
                pd: pkg:
                if (builtins.hasAttr "outputs" pd) then
                  if (builtins.isString pd.outputs) then
                    # Handle outputs = "all"
                    if (pd.outputs == "all") then
                      builtins.attrNames package.outputs
                    else
                      throw "outputs must either be 'all' or a list of output names"
                  # Handle outputs = [ "foo", "bar" ]
                  else if (builtins.isList pd.outputs) then
                    getValidAttrs pd.outputs pkg
                  else
                    throw "outputs must either be 'all' or a list of output names"
                else
                  # The problematic `stubs` outputs from CUDA packages aren't included
                  # in outputs_to_install as far as I can tell, so we don't need to
                  # filter it out here.
                  outputsToInstall
              );
              outputs =
                if (manifest.version == 1) then
                  getV1Outputs package
                else if (manifest.version == 2) then
                  getV2Outputs descriptor package
                else
                  throw "unsupported manifest version: '${manifest.version}'";
            in
            builtins.map (output: builtins.storePath (builtins.getAttr output package.outputs)) outputs
          )
        else
          [ ]
      )
    else
      [ ]
  ) lockfile.packages;

in
# Throw a more meaningful error when lockfile version is < 1.
assert (builtins.hasAttr "lockfile-version" lockfile);
assert lockfile."lockfile-version" != "0";
builtins.derivation {
  inherit name;
  builder = "${floxBuildEnv}/lib/builder.pl";
  outputs = environmentOutputs;

  # Pull in external attributes and those calculated above.
  inherit
    interpreter_out
    flox_activations_out
    interpreter_wrapper
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
    interpreter_out
    flox_activations_out
    interpreter_wrapper
    manifestPackage
  ];
}
