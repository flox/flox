#! @perl@ -w

use strict;
use Cwd 'abs_path';
use IO::Handle;
use File::Copy;
use File::Path;
use File::Basename;
use File::Compare;
use JSON::PP;
use Time::HiRes qw( gettimeofday tv_interval );

STDOUT->autoflush(1);

$SIG{__WARN__} = sub { warn "warning: ", @_ };
$SIG{__DIE__}  = sub { die "error: ", @_ };

# <flox>
# Set required ENV variables to avoid warnings.
$ENV{"pathsToLink"} = "/";
$ENV{"extraPrefix"} = "";
$ENV{"ignoreCollisions"} = "0";
$ENV{"checkCollisionContents"} = "0";

# Global variable to toggle the recursive linking of propagated-build-inputs.
my $FLOX_RECURSIVE_LINK = 0;
# </flox>

my $out = $ENV{"out"};
my $extraPrefix = $ENV{"extraPrefix"};

my @pathsToLink = split ' ', $ENV{"pathsToLink"};

sub isInPathsToLink {
    my $path = shift;
    $path = "/" if $path eq "";
    foreach my $elem (@pathsToLink) {
        return 1 if
            $elem eq "/" ||
            (substr($path, 0, length($elem)) eq $elem
             && (($path eq $elem) || (substr($path, length($elem), 1) eq "/")));
    }
    return 0;
}

# Returns whether a path in one of the linked packages may contain
# files in one of the elements of pathsToLink.
sub hasPathsToLink {
    my $path = shift;
    foreach my $elem (@pathsToLink) {
        return 1 if
            $path eq "" ||
            (substr($elem, 0, length($path)) eq $path
             && (($path eq $elem) || (substr($elem, length($path), 1) eq "/")));
    }
    return 0;
}

# Similar to `lib.isStorePath`
sub isStorePath {
    my $path = shift;
    my $storePath = "@storeDir@";

    return substr($path, 0, 1) eq "/" && dirname($path) eq $storePath;
}

# For each activated package, determine what symlinks to create.

my %symlinks;

# Add all pathsToLink and all parent directories.
#
# For "/a/b/c" that will include
# [ "", "/a", "/a/b", "/a/b/c" ]
#
# That ensures the whole directory tree needed by pathsToLink is
# created as directories and not symlinks.
$symlinks{""} = ["", 0];
for my $p (@pathsToLink) {
    my @parts = split '/', $p;

    my $cur = "";
    for my $x (@parts) {
        $cur = $cur . "/$x";
        $cur = "" if $cur eq "/";
        $symlinks{$cur} = ["", 0];
    }
}

sub findFiles;

sub findFilesInDir {
    my ($relName, $target, $ignoreCollisions, $checkCollisionContents, $priority) = @_;

    opendir DIR, "$target" or die "cannot open `$target': $!";
    my @names = readdir DIR or die;
    closedir DIR;

    foreach my $name (@names) {
        next if $name eq "." || $name eq "..";
        findFiles("$relName/$name", "$target/$name", $name, $ignoreCollisions, $checkCollisionContents, $priority);
    }
}

sub checkCollision {
    my ($path1, $path2) = @_;

    if (! -e $path1 || ! -e $path2) {
        return 0;
    }

    my $stat1 = (stat($path1))[2];
    my $stat2 = (stat($path2))[2];

    if ($stat1 != $stat2) {
        warn "different permissions in `$path1' and `$path2': "
           . sprintf("%04o", $stat1 & 07777) . " <-> "
           . sprintf("%04o", $stat2 & 07777);
        return 0;
    }

    return compare($path1, $path2) == 0;
}

sub prependDangling {
    my $path = shift;
    return (-l $path && ! -e $path ? "dangling symlink " : "") . "`$path'";
}

sub findFiles {
    my ($relName, $target, $baseName, $ignoreCollisions, $checkCollisionContents, $priority) = @_;

    # The store path must not be a file
    if (-f $target && isStorePath $target) {
        die "The store path $target is a file and can't be merged into an environment using pkgs.buildEnv!";
    }

    # Urgh, hacky...
    return if
        $relName eq "/propagated-build-inputs" ||
        $relName eq "/nix-support" ||
        $relName =~ /info\/dir$/ ||
        ( $relName =~ /^\/share\/mime\// && !( $relName =~ /^\/share\/mime\/packages/ ) ) ||
        $baseName eq "perllocal.pod" ||
        $baseName eq "log" ||
        ! (hasPathsToLink($relName) || isInPathsToLink($relName));

    my ($oldTarget, $oldPriority) = @{$symlinks{$relName} // [undef, undef]};

    # If target doesn't exist, create it. If it already exists as a
    # symlink to a file (not a directory) in a lower-priority package,
    # overwrite it.
    if (!defined $oldTarget || ($priority < $oldPriority && ($oldTarget ne "" && ! -d $oldTarget))) {
        # If target is a dangling symlink, emit a warning.
        if (-l $target && ! -e $target) {
            my $link = readlink $target;
            warn "creating dangling symlink `$out$extraPrefix/$relName' -> `$target' -> `$link'\n";
        }
        $symlinks{$relName} = [$target, $priority];
        return;
    }

    # If target already exists and both targets resolves to the same path, skip
    if (
        defined $oldTarget && $oldTarget ne "" &&
        defined abs_path($target) && defined abs_path($oldTarget) &&
        abs_path($target) eq abs_path($oldTarget)
    ) {
        # Prefer the target that is not a symlink, if any
        if (-l $oldTarget && ! -l $target) {
            $symlinks{$relName} = [$target, $priority];
        }
        return;
    }

    # If target already exists as a symlink to a file (not a
    # directory) in a higher-priority package, skip.
    if (defined $oldTarget && $priority > $oldPriority && $oldTarget ne "" && ! -d $oldTarget) {
        return;
    }

    # If target is supposed to be a directory but it isn't, die with an error message
    # instead of attempting to recurse into it, only to fail then.
    # This happens e.g. when pathsToLink contains a non-directory path.
    if ($oldTarget eq "" && ! -d $target) {
        die "not a directory: `$target'\n";
    }

    unless (-d $target && ($oldTarget eq "" || -d $oldTarget)) {
        # Prepend "dangling symlink" to paths if applicable.
        my $targetRef = prependDangling($target);
        my $oldTargetRef = prependDangling($oldTarget);

        if ($ignoreCollisions) {
            warn "collision between $targetRef and $oldTargetRef\n" if $ignoreCollisions == 1;
            return;
        } elsif ($checkCollisionContents && checkCollision($oldTarget, $target)) {
            return;
        } else {
            die "collision between $targetRef and $oldTargetRef\n";
        }
    }

    findFilesInDir($relName, $oldTarget, $ignoreCollisions, $checkCollisionContents, $oldPriority) unless $oldTarget eq "";
    findFilesInDir($relName, $target, $ignoreCollisions, $checkCollisionContents, $priority);

    $symlinks{$relName} = ["", $priority]; # denotes directory
}


my %done;
my %postponed;

sub addPkg {
    my ($pkgDir, $ignoreCollisions, $checkCollisionContents, $priority)  = @_;

    return if (defined $done{$pkgDir});
    $done{$pkgDir} = 1;

    findFiles("", $pkgDir, "", $ignoreCollisions, $checkCollisionContents, $priority);

    # <flox>
    #
    # When rendering flox develop envs treat propagated-build-inputs as
    # propagated-user-env-packages so that required packages already
    # present in the closure can be found from the one environment path.
    # This is particularly relevant for interpreted languages like python
    # that can then use a single value for PYTHONPATH rather than having
    # to rely upon walking setup hooks for constructing a long PYTHONPATH
    # during a potentially-unbounded instantiation.
    #
    if ($FLOX_RECURSIVE_LINK) {
        foreach my $propagatedFN (
            "$pkgDir/nix-support/propagated-user-env-packages", "$pkgDir/nix-support/propagated-build-inputs"
        ) {
            if (-e $propagatedFN) {
                open PROP, "<$propagatedFN" or die;
                my $propagated = <PROP>;
                close PROP;
                my @propagated = split ' ', $propagated;
                foreach my $p (@propagated) {
                    $postponed{$p} = 1 unless defined $done{$p};
                }
            }
        }
    } else {
    # </flox>

    my $propagatedFN = "$pkgDir/nix-support/propagated-user-env-packages";
    if (-e $propagatedFN) {
        open PROP, "<$propagatedFN" or die;
        my $propagated = <PROP>;
        close PROP;
        my @propagated = split ' ', $propagated;
        foreach my $p (@propagated) {
            $postponed{$p} = 1 unless defined $done{$p};
        }
    }

    # <flox>
    }
    # </flox>

}

# <flox>
if (0) {
# </flox>

# Read packages list.
my $pkgs;

if (exists $ENV{"pkgsPath"}) {
    open FILE, $ENV{"pkgsPath"};
    $pkgs = <FILE>;
    close FILE;
} else {
    $pkgs = $ENV{"pkgs"}
}

# Symlink to the packages that have been installed explicitly by the
# user.
for my $pkg (@{decode_json $pkgs}) {
    for my $path (@{$pkg->{paths}}) {
        addPkg($path,
               $ENV{"ignoreCollisions"} eq "1",
               $ENV{"checkCollisionContents"} eq "1",
               $pkg->{priority})
           if -e $path;
    }
}


# Symlink to the packages that have been "propagated" by packages
# installed by the user (i.e., package X declares that it wants Y
# installed as well).  We do these later because they have a lower
# priority in case of collisions.
my $priorityCounter = 1000; # don't care about collisions
while (scalar(keys %postponed) > 0) {
    my @pkgDirs = keys %postponed;
    %postponed = ();
    foreach my $pkgDir (sort @pkgDirs) {
        addPkg($pkgDir, 2, $ENV{"checkCollisionContents"} eq "1", $priorityCounter++);
    }
}


# Create the symlinks.
my $nrLinks = 0;
foreach my $relName (sort keys %symlinks) {
    my ($target, $priority) = @{$symlinks{$relName}};
    my $abs = "$out" . "$extraPrefix" . "/$relName";
    next unless isInPathsToLink $relName;
    if ($target eq "") {
        #print "creating directory $relName\n";
        mkpath $abs or die "cannot create directory `$abs': $!";
    } else {
        #print "creating symlink $relName to $target\n";
        symlink $target, $abs ||
            die "error creating link `$abs': $!";
        $nrLinks++;
    }
}


print STDERR "created $nrLinks symlinks in user environment\n";


my $manifest = $ENV{"manifest"};
if ($manifest) {
    symlink($manifest, "$out/manifest") or die "cannot create manifest";
}

# <flox>
} else {

    sub parseJSONFile($) {
        my $json_file = shift;
        # Read the JSON file.
        my $json = JSON::PP->new->utf8;
        open my $fh, '<', $json_file or die "Could not open file '$json_file': $!";
        local $/;  # Enable 'slurp' mode to read the whole file content at once
        my $json_text = <$fh>;
        close $fh;
        # Decode the JSON content into a Perl hash.
        return $json->decode($json_text);
    }

    # Process the manifest data to produce an array of package objects
    # compatible with the "pkgs" variable as found in the original code.
    sub outputData($$) {
        my $nix_attrs = shift;
        my $manifestData = shift;

        # Function for emitting a package set in the format consumed by
        # the builder.pl script.
        sub packagesToPkgs($) {
            my $packages = shift;
            my @retarray = ();
            foreach my $package (@{$packages}) {
                my @storePaths = ();
                # XXX flake locking bug: outputs-to-install != outputs_to_install
                if ( defined $package->{"outputs-to-install"} and not
                     defined $package->{"outputs_to_install"} ) {
                    $package->{"outputs_to_install"} = $package->{"outputs-to-install"};
                }
                foreach my $output (keys %{$package->{"outputs"}}) {
                    next unless grep { $_ eq $output } @{$package->{"outputs_to_install"}};
                    push @storePaths, $package->{"outputs"}{$output};
                }
                next unless scalar @storePaths;
                push @retarray, {
                    "paths" => \@storePaths,
                    "priority" => $package->{"priority"}
                };
            }
            return \@retarray;
        }

        # We can have nice names for things.
        my $activationScripts = $nix_attrs->{"activationScripts"};
        my $manifestPackage = $nix_attrs->{"manifestPackage"};
        my $system = $nix_attrs->{"system"};
        my $packages = $manifestData->{"packages"};
        my $manifest = $manifestData->{"manifest"};
        my $install = $manifest->{"install"};
        my $builds = $manifest->{"build"};
        my @buildNames = keys %{$builds};

        # Construct an array containing the Flox activation-scripts packages.
        my @floxEnvironmentPackages = (
            {
                "group" => "toplevel", # Want to appear in build closures.
                "outputs_to_install" => [ "out" ],
                "outputs" => {
                    "out" => $activationScripts
                },
                priority => 1
            },
            {
                "group" => "toplevel", # Want to appear in build closures.
                "outputs_to_install" => [ "out" ],
                "outputs" => {
                    "out" => $manifestPackage
                },
                priority => 1
            },
        );

        # Filter system-specific outputs to include in the "out" output.
        my @outPackages = grep { $_->{"system"} eq $system } @{$packages};

        # Define the "develop" output as all packages with activation scripts included.
        my @developPackages = ( @outPackages, @floxEnvironmentPackages );

        # Filter only packages included in the "toplevel" group for use in builds.
        my @toplevelPackages = grep { $_->{"group"} eq "toplevel" } @developPackages;

        my %buildPackagesHash = ();
        if (scalar @buildNames) {
            # Each build gets its own output closure including packages
            # selected from the @toplevelPackages set. If the "packages"
            # attribute is not defined then the build will use all of
            # @toplevelPackages.
            foreach my $build (@buildNames) {
                # Come up with the list of candidate package installation names
                # to be installed.
                if (defined $builds->{$build}{"packages"}) {
                    my @buildPackageNames = @{$builds->{$build}{"packages"}};
                    # Derive the corresponding package attr-paths.
                    my @buildPackageAttrPaths;
                    foreach my $name (@buildPackageNames) {
                        if (exists $install->{$name}) {
                            push @buildPackageAttrPaths, $install->{$name}{"pkg-path"};
                        }
                    }
                    # Filter packages found in the "toplevel" pkg-group to include only
                    # those packages found in `$buildPackageAttrPaths`.
                    my @buildPackages;
                    foreach my $package (@toplevelPackages) {
                        if (grep { $_ eq $package->{"attr_path"} } @buildPackageAttrPaths) {
                            push @buildPackages, $package;
                        }
                    }
                    # Represent the result as a hash keyed by the build name.
                    $buildPackagesHash{$build} = [ @buildPackages, @floxEnvironmentPackages ];
                } else {
                    $buildPackagesHash{$build} = \@toplevelPackages;
                }
            }
        }

        # Construct data sets for each environment to be rendered by the
        # builder.pl script.
        my @outputData = (
            {
              "name" => "out",
              "pkgs" => packagesToPkgs(\@developPackages),
              "recurse" => 0
            },
            {
              "name" => "develop",
              "pkgs" => packagesToPkgs(\@developPackages),
              "recurse" => 1
            }
        );
        foreach my $buildName (@buildNames) {
            push @outputData, {
                "name" => "build-$buildName",
                "pkgs" => packagesToPkgs($buildPackagesHash{$buildName}),
                "recurse" => 1
            };
        }

        return \@outputData;
    }

    sub buildEnv($$$$) {
        my $envName = shift;
        my $requisites = shift;
        my $out = shift;
        my $pkgs = shift;
        my $t0 = [gettimeofday];

        # Flox: the remainder of this function is copied from above.

        # Symlink to the packages that have been installed explicitly by the
        # user.
        for my $pkg (@{$pkgs}) {
            for my $path (@{$pkg->{paths}}) {
                addPkg($path,
                       $ENV{"ignoreCollisions"} eq "1",
                       $ENV{"checkCollisionContents"} eq "1",
                       $pkg->{priority})
                   if -e $path;
            }
        }

        # Symlink to the packages that have been "propagated" by packages
        # installed by the user (i.e., package X declares that it wants Y
        # installed as well).  We do these later because they have a lower
        # priority in case of collisions.
        my $priorityCounter = 1000; # don't care about collisions
        while (scalar(keys %postponed) > 0) {
            my @pkgDirs = keys %postponed;
            %postponed = ();
            foreach my $pkgDir (sort @pkgDirs) {
                addPkg($pkgDir, 2, $ENV{"checkCollisionContents"} eq "1", $priorityCounter++);
            }
        }

        # Create the symlinks.
        my $nrLinks = 0;
        foreach my $relName (sort keys %symlinks) {
            my ($target, $priority) = @{$symlinks{$relName}};
            my $abs = "$out" . "$extraPrefix" . "/$relName";
            next unless isInPathsToLink $relName;
            if ($target eq "") {
                #print "creating directory $relName\n";
                mkpath $abs or die "cannot create directory `$abs': $!";
            } else {
                #print "creating symlink $relName to $target\n";
                symlink $target, $abs ||
                    die "error creating link `$abs': $!";
                $nrLinks++;
            }
        }

        printf STDERR "created $nrLinks symlinks in $envName environment in %.06f seconds\n", tv_interval ( $t0 );

        unless ( -e "$out" ) {
            mkdir $out or die "cannot create directory `$out': $!";
        }

        # Walk the %{$requisites} data for each package in the %done hash
        # populating the %requisites_txt hash.
        my %requisites_txt = ();
        foreach my $key (keys %done) {
            foreach my $requisite (@{$requisites->{$key}}) {
                $requisites_txt{$requisite} = 1;
            }
        }

        # Make sure the package itself is included in its requisites.
        $requisites_txt{$out} = 1;

        # Write sorted requisites to $out/requisites.txt.
        my $file = "$out/requisites.txt";
        open(my $fh, '>', $file) or die "Could not open file '$file' $!";
        foreach my $requisite (sort keys %requisites_txt) {
            print $fh "$requisite\n";
        }
        # Close the file
        close $fh or die "Could not close file '$file' $!";
    }

    # Avoid the use of "pkgs" and "pkgsPath" env variables by instead
    # directly reading the $NIX_ATTRS_JSON_FILE.
    die "NIX_ATTRS_JSON_FILE not defined"
        unless defined $ENV{"NIX_ATTRS_JSON_FILE"};
    my $nix_attrs = parseJSONFile($ENV{"NIX_ATTRS_JSON_FILE"});
    my $manifestData = parseJSONFile($nix_attrs->{"manifestPackage"} . "/manifest.lock");

    # Construct outputData from the manifest.
    my $outputData = outputData($nix_attrs, $manifestData);

    # The Nix exportReferencesGraph attribute directs Nix to dump the closure
    # for each input package into the $nix_attrs->{'exportReferencesGraph'}
    # hash. Walk this information to construct a %references hash for use when
    # populating requisites.txt.
    #
    # Convert array of:
    #
    # {
    #   'path': '/nix/store/foo',
    #   'references': [ '/nix/store/bar', '/nix/store/baz' ]
    # },
    # {
    #   'path': '/nix/store/baz',
    #   'references': [ '/nix/store/bomb', '/nix/store/buzz' ]
    # },
    # ...
    #
    # into hash of:
    #
    # {
    #   '/nix/store/foo': [ '/nix/store/bar', '/nix/store/baz' ],
    #   '/nix/store/baz': [ '/nix/store/bomb', '/nix/store/buzz' ],
    #   ...
    # }
    sub mapReferences($) {
        my $pkgs = shift @_;
        my %references = ();
        foreach my $pkg (@{$pkgs}) {
            $references{$pkg->{'path'}} = $pkg->{'references'};
        }
        return \%references;
    }

    # Walk reference tree as above, recursively returning references.
    sub walkReferences($$);
    sub walkReferences($$) {
        my $references = shift @_;
        my $pkg = shift @_;
        my @retarray = ( $pkg );
        if (defined $references->{$pkg}) {
            foreach my $reference (@{$references->{$pkg}}) {
                next if $reference eq $pkg;
                push @retarray, walkReferences($references, $reference);
            }
        } else {
            warn "references for package $pkg not found\n";
        }
        return @retarray;
    }

    # Populate requisites graph used for all outputs.
    my %requisites = ();
    foreach my $graphName (%{$nix_attrs->{'exportReferencesGraph'}}) {
        my $references = mapReferences($nix_attrs->{$graphName});
        foreach my $pkg (@{$nix_attrs->{'exportReferencesGraph'}{$graphName}}) {
            foreach my $reference (walkReferences($references, $pkg)) {
                push @{$requisites{$pkg}}, $reference;
            }
        }
    }

    # Iterate over $outputData creating the symlink trees.
    foreach my $output (@{$outputData}) {
        # Wipe out global state.
        %done = ();
        %postponed = ();
        %symlinks = ();
        my $envName = $output->{"name"};

        my $path = $nix_attrs->{"outputs"}{$envName};
        my $pkgs = $output->{"pkgs"};
        $FLOX_RECURSIVE_LINK = ( $output->{"recurse"} eq "1" ) ? 1 : 0;
        buildEnv($envName, \%requisites, $path, $pkgs);
    }
}
# </flox>
