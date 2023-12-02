/* ========================================================================== *
 *
 * @file logger.cc
 *
 * @brief Custom `nix::Logger` implementation used to filter some messages.
 *
 *
 * -------------------------------------------------------------------------- */

#include <nix/logging.hh>
#include <nix/util.hh>
#include <optional>
#include <string>

#include "flox/core/nix-state.hh"


/* -------------------------------------------------------------------------- */

namespace flox {

/* -------------------------------------------------------------------------- */

/**
 * @brief determine if we should use ANSI escape sequences.
 *
 * This is a copy of `nix::shouldANSI` with the addition of checking the
 * `NOCOLOR` environment variable ( `nix::shouldANSI` only checks `NO_COLOR` ).
 */
static bool
shouldANSI()
{
  return isatty( STDERR_FILENO )
         && ( nix::getEnv( "TERM" ).value_or( "dumb" ) != "dumb" )
         && ( ! ( nix::getEnv( "NO_COLOR" ).has_value()
                  || nix::getEnv( "NOCOLOR" ).has_value() ) );
}


/* -------------------------------------------------------------------------- */

/**
 * @brief Custom `nix::Logger` implementation used to filter some messages.
 *
 * This is an exact copy of `nix::SimpleLogger` with the addition of filtering
 * in the `log` routine.
 */
class FilteredLogger : public nix::Logger
{

protected:

  /**
   * @brief Detect ignored warnings.
   *
   * In theory this is normally controlled by verbosity, but because the
   * verbosity setting conditionals exist in external libs, we have to
   * handle them here.
   */
  bool
  shouldIgnoreWarning( const std::string & str )
  {
    /* Ignore warnings about overrides for missing indirect inputs.
     * These can come up when an indirect input drops a dependendency
     * between different revisions and isn't particularly interesting
     * to users. */
    if ( str.find( " has an override for a non-existent input " )
         != std::string::npos )
      {
        /* Don't ignore with `-v' or if we are dumping logs to a file. */
        return ( ! this->tty ) || ( nix::verbosity < nix::lvlTalkative );
      }

    return false;
  }


  /** @brief Detect ignored messages. */
  bool
  shouldIgnoreMsg( std::string_view str )
  {
    (void) str;
    return false;
  }


public:

  bool systemd;        /**< Whether we should emit `systemd` style logs. */
  bool tty;            /**< Whether we are connected to a TTY. */
  bool color;          /**< Whether we should emit colors in logs. */
  bool printBuildLogs; /**< Whether we should emit build logs. */

  FilteredLogger( bool printBuildLogs )
    : systemd( nix::getEnv( "IN_SYSTEMD" ) == "1" )
    , tty( isatty( STDERR_FILENO ) )
    , color( shouldANSI() )
    , printBuildLogs( printBuildLogs )
  {}


  /** @brief Whether the logger prints the whole build log. */
  bool
  isVerbose() override
  {
    return this->printBuildLogs;
  }


  /** @brief Emit a log message with a colored "warning:" prefix. */
  void
  warn( const std::string & msg ) override
  {
    if ( ! this->shouldIgnoreWarning( msg ) )
      {
        /* NOTE: The `nix' definitions of `ANSI_WARNING' and `ANSI_NORMAL'
         *       use `\e###` escapes, but `gcc' will gripe at you for not
         *       following ISO standard.
         *       We use equivalent `\033###' sequences instead.' */
        this->log( nix::lvlWarn,
                   /* ANSI_WARNING */ "\033[35;1m"
                                      "warning:"
                                      /* ANSI_NORMAL */ "\033[0m"
                                      " "
                     + msg );
      }
  }


  /**
   * @brief Emit a log line depending on verbosity setting.
   * @param lvl Minimum required verbosity level to emit the message.
   * @param str The message to emit.
   */
  void
  log( nix::Verbosity lvl, std::string_view str ) override
  {
    if ( ( nix::verbosity < lvl ) || this->shouldIgnoreMsg( str ) ) { return; }

    /* Handle `systemd' style log level prefixes. */
    std::string prefix;
    if ( systemd )
      {
        char levelChar;
        switch ( lvl )
          {
            case nix::lvlError: levelChar = '3'; break;

            case nix::lvlWarn: levelChar = '4'; break;

            case nix::lvlNotice:
            case nix::lvlInfo: levelChar = '5'; break;

            case nix::lvlTalkative:
            case nix::lvlChatty: levelChar = '6'; break;

            case nix::lvlDebug:
            case nix::lvlVomit: levelChar = '7'; break;

            /* Should not happen, and missing enum case is reported
             * by `-Werror=switch-enum' */
            default: levelChar = '7'; break;
          }
        prefix = std::string( "<" ) + levelChar + ">";
      }

    nix::writeToStderr( prefix + nix::filterANSIEscapes( str, ! this->color )
                        + "\n" );
  }


  /** @brief Emit error information. */
  void
  logEI( const nix::ErrorInfo & einfo ) override
  {
    std::stringstream oss;
    /* From `nix/error.hh' */
    showErrorInfo( oss, einfo, nix::loggerSettings.showTrace.get() );

    this->log( einfo.level, oss.str() );
  }


  /** @brief Begin an activity block. */
  void
  startActivity( nix::ActivityId /* act ( unused ) */
                 ,
                 nix::Verbosity lvl,
                 nix::ActivityType /* type ( unused ) */
                 ,
                 const std::string & str,
                 const Fields & /* fields ( unused ) */
                 ,
                 nix::ActivityId /* parent ( unused ) */
                 ) override
  {
    if ( ( lvl <= nix::verbosity ) && ( ! str.empty() ) )
      {
        this->log( lvl, str + "..." );
      }
  }


  /** @brief Report the result of an RPC call with a remote `nix` store. */
  void
  result( nix::ActivityId /* act ( unused ) */
          ,
          nix::ResultType type,
          const Fields &  fields ) override
  {
    if ( ! this->printBuildLogs ) { return; }
    if ( type == nix::resBuildLogLine )
      {
        this->log( nix::lvlError, fields[0].s );
      }
    else if ( type == nix::resPostBuildLogLine )
      {
        this->log( nix::lvlError, "post-build-hook: " + fields[0].s );
      }
  }


}; /* End class `FilteredLogger' */


/* -------------------------------------------------------------------------- */

nix::Logger *
makeFilteredLogger( bool printBuildLogs )
{
  return new FilteredLogger( printBuildLogs );
}


/* -------------------------------------------------------------------------- */

}  // namespace flox


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
