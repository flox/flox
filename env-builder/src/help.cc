/* ========================================================================== *
 *
 * @file help.cc
 *
 * @brief Routines used to produce help messages and `flox help`
 * subcommand implementation.
 *
 *
 * -------------------------------------------------------------------------- */

#include <nix/command.hh>

#include "flox/command.hh"


/* -------------------------------------------------------------------------- */

namespace flox {

/* -------------------------------------------------------------------------- */

  static void
showUsageTop( std::ostream & fd, nix::MultiCommand & toplevel )
{
  fd << "Usage: flox OPTIONS... (";
  for ( auto & [name, commandFun] : toplevel.commands )
    {
      auto command = commandFun();
      /* Only print "popular" commands in usage. */
      switch ( command->category() )
        {
          case nix::Command::catDefault: break;
          case flox::catLocal:           break;
          case flox::catSharing:         break;
          /* Skip everyting else. */
          case nix::catHelp:        continue; break;
          case flox::catAdditional: continue; break;
          default:                  continue; break;
        }
      fd << name << '|';
    }
  fd << "...) [--help]" << std::endl;
}


/* -------------------------------------------------------------------------- */

  void
showSubcommandHelp( std::ostream & fd, nix::MultiCommand & command )
{
  showUsageTop( fd, command );

  /* Show Options */
  fd << std::endl << "OPTIONS" << std::endl;
  std::unordered_map<std::string, std::string> optFlags;
  size_t width = 0;
  nlohmann::json j = command.toJSON();
  if ( j.find( "flags" ) == j.end() )
    {
      j["flags"] = nlohmann::json::object();
    }
  for ( auto & [name, flag] : command.toJSON().at( "flags" ).items() )
    {
      std::string lhs( "--" + name );
      auto mShort = flag.find( "shortName" );
      if ( mShort != flag.end() )
        {
          lhs += ",-";
          lhs += mShort.value();
        }
      lhs += ' ';
      lhs += nix::concatStringsSep(
               " "
             , (std::vector<std::string>) flag.at( "labels" )
             );
      /* Find the longest so we can align. */
      if ( width < lhs.size() ) { width = lhs.size(); }
      optFlags.emplace( name, lhs );
    }
  /* Now that we know the longest "left has side", we can print. */
  for ( auto & [name, lhs] : optFlags )
    {
      std::ostringstream oss;
      oss << std::left << std::setfill( ' ' ) << std::setw( width ) << lhs;
      fd << "  " << oss.str() << "  "
         << j.at( name ).at( "description" ) << std::endl;
    }

  /* Show Commands */
  fd << std::endl << "COMMANDS" << std::endl;
  /* Get the widest subcommand name in the categories we show. */
  width = 0;
  for ( auto & [name, commandFun] : command.commands )
    {
      switch ( commandFun()->category() )
        {
          case nix::Command::catDefault: break;
          case flox::catLocal:           break;
          case flox::catSharing:         break;
          /* Skip everyting else. */
          case nix::catHelp:        continue; break;
          case flox::catAdditional: continue; break;
          default:                  continue; break;
        }
      if ( width < name.size() ) { width = name.size(); }
    }
  for ( auto & [category, desc] : command.categories )
    {
      /* Don't print the "Help commands" category */
      if ( category == nix::catHelp ) { continue; }

      fd << "  " << desc;
      if ( category == flox::catAdditional )
        {
          fd << ". Use `flox COMMAND --help` for more info" << std::endl;
          bool   first = true;
          size_t count = 0;
          for ( auto & [name, commandFun] : command.commands )
            {
              if ( commandFun()->category() == flox::catAdditional )
                {
                  count += 2 + name.size();
                  if ( first )
                    {
                      first = false;
                      count = 4 + name.size();
                      fd << "    ";
                      assert( count <= 80 );
                    }
                  else if ( 80 < count )
                    {
                      fd << ',' << std::endl << "    ";
                      count = 4 + name.size();
                    }
                  else
                    {
                      fd << ", ";
                    }
                  fd << name;
                }
            }
          fd << std::endl;
        }
      else
        {
          fd << std::endl;
          for ( auto & [name, commandFun] : command.commands )
            {
              auto command = commandFun();
              /* Only print "popular" commands in usage. */
              if ( command->category() != category ) { continue; }
              std::ostringstream oss;
              oss << std::left << std::setfill( ' ' ) << std::setw( width )
                  << name;
              fd << "    " << oss.str() << "  " << command->description()
                 << std::endl;
            }
        }
      fd << std::endl;
    }
}


/* -------------------------------------------------------------------------- */

  static void
showSubcommandUsage( std::ostream      & fd
                   , std::string_view    name
                   , nix::MultiCommand & command
                   )
{
  fd << "Usage: flox " << name << "OPTIONS... (";
  for ( auto & [name, commandFun] : command.commands )
    {
      auto command = commandFun();
      /* Only print "popular" commands in usage. */
      switch ( command->category() )
        {
          case nix::Command::catDefault: break;
          case flox::catLocal:           break;
          case flox::catSharing:         break;
          /* Skip everyting else. */
          case nix::catHelp:        continue; break;
          case flox::catAdditional: continue; break;
          default:                  continue; break;
        }
      fd << name << '|';
    }
  fd << "...) [--help]" << std::endl;
}


/* -------------------------------------------------------------------------- */

/**
 * Render the help for the specified subcommand to stdout using
 * lowdown.
 */
  void
showHelp( std::vector<std::string> subcommand, FloxArgs & toplevel )
{
  showSubcommandHelp( std::cout, toplevel );
  // TODO
}


/* -------------------------------------------------------------------------- */

  static FloxArgs &
getFloxArgs( nix::Command & cmd )
{
  assert( cmd.parent );
  /* Find the "top level" command by traversing parents. */
  nix::MultiCommand * toplevel = cmd.parent;
  while ( toplevel->parent != nullptr ) { toplevel = toplevel->parent; }
  return dynamic_cast<FloxArgs &>( * toplevel );
}


/* -------------------------------------------------------------------------- */

struct CmdHelp : nix::Command
{
  std::vector<std::string> subcommand;

  CmdHelp()
  {
    expectArgs( {
      .label   = "subcommand"
    , .handler = { & subcommand }
    } );
  }

    std::string
  description() override
  {
    return "show help about `flox` or a particular subcommand";
  }

  std::string doc() override { return "TODO"; }

  nix::Command::Category category() override { return flox::catAdditional; }

  void run() override
  {
    assert( parent );
    nix::MultiCommand * toplevel = parent;
    while ( toplevel->parent ) { toplevel = toplevel->parent; }
    showHelp( subcommand, getFloxArgs( * this ) );
  }

};

static auto rCmdHelp = nix::registerCommand<CmdHelp>( "help" );


/* -------------------------------------------------------------------------- */

}  /* End namespace `flox' */


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
