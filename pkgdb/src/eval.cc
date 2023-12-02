/* ========================================================================== *
 *
 * @file eval.hh
 *
 * @brief Executable command helpers, argument parsers, etc.
 *
 *
 * -------------------------------------------------------------------------- */

#include <fstream>

#include <nix/eval.hh>
#include <nix/value-to-json.hh>

#include "flox/eval.hh"


/* -------------------------------------------------------------------------- */

namespace flox {

/* -------------------------------------------------------------------------- */

EvalCommand::EvalCommand() : parser( "eval" )
{
  this->parser.add_description(
    "Evaluate a `nix` expression with `flox` extensions" );

  this->parser.add_argument( "--json", "-j" )
    .help( "emit JSON values" )
    .nargs( 0 )
    .action(
      [&]( const auto & )
      {
        if ( this->style == STYLE_RAW )
          {
            throw FloxException(
              "the options `--json' and `--raw' may not be used together" );
          }
        this->style = STYLE_JSON;
      } );

  this->parser.add_argument( "--raw", "-r" )
    .help( "emit strings without quotes" )
    .nargs( 0 )
    .action(
      [&]( const auto & )
      {
        if ( this->style == STYLE_JSON )
          {
            throw FloxException(
              "the options `--json' and `--raw' may not be used together" );
          }
        this->style = STYLE_RAW;
      } );

  this->parser.add_argument( "--file", "-f" )
    .help( "read expression from a file. "
           "Use `-' as filename to read `STDIN'" )
    .nargs( 1 )
    .metavar( "FILE" )
    .action(
      [&]( const std::string & file )
      {
        if ( this->expr.has_value() )
          {
            throw FloxException(
              "the option `--file' may not be used with an inline expression" );
          }
        this->file = file;
      } );

  this->parser.add_argument( "--impure", "-i" )
    .help( "allow impure evaluation" )
    .nargs( 0 )
    .action( [&]( const auto & ) { nix::evalSettings.pureEval = false; } );

  this->parser.add_argument( "expr" )
    .help( "expression to evaluate" )
    .nargs( argparse::nargs_pattern::optional )
    .metavar( "EXPR" )
    .action(
      [&]( const std::string & expr )
      {
        if ( this->file.has_value() )
          {
            throw FloxException(
              "the option `--file' may not be used with an inline expression" );
          }
        this->expr = expr;
      } );
}


/* -------------------------------------------------------------------------- */

int
EvalCommand::run()
{
  auto state = this->getState();
  auto value = state->allocValue();
  if ( this->file.has_value() )
    {
      if ( ( *this->file ) == "-" )
        {
          auto expr = state->parseStdin();
          state->eval( expr, *value );
        }
      else
        {
          state->evalFile(
            state->rootPath( nix::CanonPath( this->file->string() ) ),
            *value );
        }
    }
  else
    {
      if ( ! this->expr.has_value() )
        {
          throw FloxException(
            "you must provide a file or expression to evaluate" );
        }
      auto expr = state->parseExprFromString(
        *this->expr,
        state->rootPath( nix::CanonPath::fromCwd() ) );
      state->eval( expr, *value );
    }

  nix::NixStringContext context;

  switch ( this->style )
    {
      case STYLE_VALUE:
        state->forceValueDeep( *value );
        nix::logger->cout( "%s", nix::printValue( *state, *value ) );
        break;

      case STYLE_RAW:
        nix::writeFull(
          STDOUT_FILENO,
          *state->coerceToString( nix::noPos,
                                  *value,
                                  context,
                                  "while generate eval command output" ) );
        break;

      case STYLE_JSON:
        nix::logger->cout( "%s",
                           nix::printValueAsJSON( *state,
                                                  true,
                                                  *value,
                                                  nix::noPos,
                                                  context,
                                                  false ) );
    }

  return EXIT_SUCCESS;
}


/* -------------------------------------------------------------------------- */

}  // namespace flox


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
