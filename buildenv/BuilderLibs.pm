# <flox>
# We split all perl lib imports out into its own module so that it can be used
# as the basis for (re)building the flox-perl package only as necessary.
# </flox>
package BuilderLibs;

use warnings;
use Exporter ();
use Exporter::Heavy ();

our @ISA = qw(Exporter);

our @EXPORT = qw(
    abs_path
    basename
    compare
    dirname
    gettimeofday
    mkpath
    tv_interval
);

use lib ();
use Cwd ();
use IO::Handle ();
use File::Copy ();
use File::Path ();
use File::Basename ();
use File::Compare ();
use JSON::PP ();
use Time::HiRes ();

*abs_path     = \&Cwd::abs_path;
*basename     = \&File::Basename::basename;
*compare      = \&File::Compare::compare;
*dirname      = \&File::Basename::dirname;
*gettimeofday = \&Time::HiRes::gettimeofday;
*mkpath       = \&File::Path::mkpath;
*tv_interval  = \&Time::HiRes::tv_interval;

sub import {
    strict->import;
    warnings->import;
    __PACKAGE__->export_to_level(1, @_);
}

1;
