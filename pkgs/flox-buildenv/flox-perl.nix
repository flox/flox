{
  lib,
  perl,
  perlScript, # Script which determines the modules to keep.
  stdenv,
}:
let
  # Would like to disable the building of _all_ unnecessary extensions but
  # found it was altogether too easy to break the build. This approach lets
  # us avoid building extensions that we know we won't need, and then later
  # we can use the "profiling" approach to pare back the package even further.
  #
  # Be sure to comment _out_ extensions we need in the list below.
  noExtensions = [
    "B"
    "Compress/Raw/Bzip2"
    "Compress/Raw/Zlib"
    # "Cwd" # Required by builder.pl.
    "DB_File"
    # "Data/Dumper" # Desired for debugging.
    "Devel/PPPort"
    "Devel/Peek"
    "Digest/MD5"
    "Digest/SHA"
    "Encode"
    "Fcntl"
    "File/DosGlob"
    # "File/Glob" # Required for `./perl -Ilib -I. installperl`.
    "Filter/Util/Call"
    "Hash/Util"
    "Hash/Util/FieldHash"
    "I18N/Langinfo"
    # "IO" # Required by builder.pl for IO/Handle.pm.
    "IPC/SysV"
    # "List/Util" # Required for Scalar/Util.pm.
    "MIME/Base64"
    "Math/BigInt/FastCalc"
    "NDBM_File"
    "Opcode"
    "POSIX"
    "PerlIO/encoding"
    "PerlIO/mmap"
    "PerlIO/via"
    "SDBM_File"
    "Socket"
    "Storable"
    "Sys/Hostname"
    "Sys/Syslog"
    # "Time/HiRes" # Required by builder.pl.
    "Time/Piece"
    "Unicode/Collate"
    "Unicode/Normalize"
    "XS/APItest"
    "XS/Typemap"
    "attributes"
    "mro"
    "re" # Required when building pods, but we have disabled.
    "threads"
    "threads/shared"
  ];

  # The script which determines the modules to keep.
  perlExerciseModules = [
    # Require perl5db.pl and Term/ReadLine.pm so that we can invoke the debugger.
    "require \"perl5db.pl\""
    "require Term::ReadLine"
    # Require Data/Dumper.pm so that we can inspect data from the debugger.
    "require Data::Dumper"
    # Pull in modules required by the supplied ${perlScript}.
    "do \"${perlScript}\""
  ];
  perlExerciseModulesCommands = builtins.concatStringsSep "; " perlExerciseModules;

in
perl.overrideAttrs (oldAttrs: {
  pname = "flox-perl";
  # No need for man or devdoc outputs.
  outputs = [ "out" ];

  # Update configureFlags to minimize build.
  configureFlags = oldAttrs.configureFlags ++ [
    # Disable shared libperl
    "-Uuseshrplib"
    # Optimize for size
    "-Doptimize=-Os"
    # No man pages
    "-Uman1dir"
    "-Uman3dir"
    # Disable building of unnecessary extensions.
    "-Dnoextensions='${lib.concatStringsSep " " noExtensions}'"
  ];

  # Disable building and installation of pods, strip binaries.
  makeFlags = (oldAttrs.makeFlags or [ ]) ++ [
    "generated_pods=" # Disable building of pods.
    "INSTALLFLAGS=-p" # Don't attempt to install the pod files.
    "STRIPFLAGS=-s" # Run strip on installed binaries.
  ];

  # Testing
  doCheck = false;

  postInstall = ''
    # The upstream hook depends upon $man being defined.
    man="/no-such-path"
  ''
  + oldAttrs.postInstall

  # Can remove once https://github.com/NixOS/nixpkgs/pull/386700
  # has flowed through to our build (hence the use of --replace-quiet).
  + (lib.optionalString ((stdenv.cc.fallback_sdk or null) != null) ''
    substituteInPlace "$out"/lib/perl5/*/*/Config_heavy.pl \
      --replace-quiet "${stdenv.cc.fallback_sdk}" /no-such-path;
  '')

  + ''
    (
      set -x

      # Remove dependencies in Config_heavy.pl.
      sed -e '/incpth=/s/.nix.store.[^-]*-/\/no-such-path-/g' \
        -e 's/\/nix\/store\/[^/]*-coreutils-[^/]*\/bin\///g' \
        -i "$out"/lib/perl5/*/*/Config_heavy.pl

      # Remove dependency on coreutils in Cwd.pm.
      sed -e 's/\/nix\/store\/[^/]*-coreutils-[^/]*//g' \
        -i "$out"/lib/perl5/*/*/Cwd.pm

      # Move over all modules required by the build. The following command
      # prints out all module files exercised by way of the command, which
      # itself exercises all of the same inputs used by builder.pl.
      keeplibs=$(mktemp)
      $out/bin/perl -MConfig \
        -e '${perlExerciseModulesCommands}; END {
          foreach my $lib (keys %INC) {
            next if $lib eq "${perlScript}";
            if ( -e "$Config{archlib}/$lib" ) {
              print "$Config{version}/$Config{archname}/$lib\n";
              my $libBasename = $lib;
              $libBasename =~ s/\.pm$//;
              if ( -d "$Config{archlib}/auto/$libBasename" ) {
                print "$Config{version}/$Config{archname}/auto/$libBasename\n";
              }
            } else {
              print "$Config{version}/$lib\n";
            }
          };
          print "$Config{version}/$Config{archname}/Config_heavy.pl\n";
        }' > $keeplibs

      # Create new lib directory.
      mv $out/lib/perl5 $out/lib/perl5.orig
      mkdir $out/lib/perl5
      tar -C $out/lib/perl5.orig -cf - --files-from $keeplibs | tar -C $out/lib/perl5 -xvf -
      rm -rf $out/lib/perl5.orig

      # Create new bin directory.
      mv $out/bin $out/bin.orig
      mkdir $out/bin
      mv $out/bin.orig/perl $out/bin
      rm -rf $out/bin.orig
    )
  '';

  postFixup = ''
    (
      set -x
      # Remove nix-support.
      rm -rf $out/nix-support
    )
  '';

  # Check only the functionality we actually need.
  doInstallCheck = true;
  installCheckPhase = ''
    # The following command exercises the same modules used by builder.pl.
    (
      set -x
      $out/bin/perl -e '${perlExerciseModulesCommands}'
    )
  '';
})
