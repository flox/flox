/* ========================================================================== *
 *
 * @file parse/command.cc
 *
 * @brief Executable command helpers, argument parsers, etc.
 *
 *
 * -------------------------------------------------------------------------- */

#include "flox/parse/command.hh"


/* -------------------------------------------------------------------------- */

namespace flox::parse {

/* -------------------------------------------------------------------------- */

DescriptorCommand::DescriptorCommand() : parser( "descriptor" )
{
  this->parser.add_description( "Parse a package descriptor" );
  this->parser.add_argument( "descriptor" )
    .help( "a package descriptor to parse" )
    .metavar( "DESCRIPTOR" )
    .action( [&]( const std::string & desc )
             { this->descriptor = resolver::ManifestDescriptor( desc ); } );
  this->parser.add_argument( "-t", "--to" )
    .help(
      "output format of parsed descriptor ['manifest' (default), 'query']" )
    .metavar( "FORMAT" )
    .nargs( 1 )
    .action( [&]( const std::string & format ) { this->format = format; } );
}


/* -------------------------------------------------------------------------- */

int
DescriptorCommand::run()
{
  nlohmann::json output;
  if ( this->format == "manifest" )
    {
      resolver::to_json( output, this->descriptor );
    }
  else if ( this->format == "query" )
    {
      pkgdb::PkgQueryArgs args;
      this->descriptor.fillPkgQueryArgs( args );
      pkgdb::to_json( output, args );
    }
  else
    {
      throw flox::FloxException( "unrecognized format: `" + this->format
                                 + "'" );
      return EXIT_FAILURE;
    }
  std::cout << output << std::endl;
  return EXIT_SUCCESS;
}


/* -------------------------------------------------------------------------- */

ParseCommand::ParseCommand() : parser( "parse" )
{
  this->parser.add_description( "Parse various constructs" );
  this->parser.add_subparser( this->cmdDescriptor.getParser() );
}


/* -------------------------------------------------------------------------- */

int
ParseCommand::run()
{
  if ( this->parser.is_subcommand_used( "descriptor" ) )
    {
      return this->cmdDescriptor.run();
    }
  std::cerr << this->parser << std::endl;
  throw flox::FloxException( "You must provide a valid `parse' subcommand" );
  return EXIT_FAILURE;
}


/* -------------------------------------------------------------------------- */

}  // namespace flox::parse


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
