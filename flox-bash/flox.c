/*
 * flox wrapper - set environment variables prior to launching flox
 */

#include <stdlib.h>
#include <unistd.h>
#include <string.h>
#include <stdarg.h>
#include <stdio.h>
#include <err.h>
#include <syslog.h>

#define LOG_STRERROR ": %m"

/*
 * Print and log a fatal error message (including a system error), and die.
 */
static void __attribute__((noreturn, format(printf, 1, 2)))
fatal(const char *format, ...)
{
	va_list ap;

	size_t len = strlen(format);
	char *sformat = alloca(len + sizeof(LOG_STRERROR));
	strcpy(sformat, format);
	strcpy(sformat + len, LOG_STRERROR);

	va_start(ap, format);
	vsyslog(LOG_ERR, sformat, ap);
	va_end(ap);

	va_start(ap, format);
	verr(EXIT_FAILURE, format, ap);
	va_end(ap);
}


int
main(int argc, char **argv)
{
	/*
	 * Nixpkgs itself is broken in that the packages it creates depends
	 * upon a variety of environment variables at runtime.  On NixOS
	 * these are convenient to set on a system-wide basis but that
	 * essentially masks the problem, and it's not uncommon to see Nix
	 * packages trip over the absence of environment variables when
	 * invoked on other Linux distributions.
	 *
	 * For flox specifically, set Nix-provided defaults for certain
	 * environment variables that we know to be required on the various
	 * operating systems.
	 */
	char *envVar;
	envVar = getenv("SSL_CERT_FILE");
	if (envVar == NULL) {
		envVar = NIXPKGS_CACERT_BUNDLE_CRT;
		if (setenv("SSL_CERT_FILE", envVar, 1) != 0)
			fatal("setenv");
	}
	if (getenv("NIX_SSL_CERT_FILE") == NULL) {
		if (setenv("NIX_SSL_CERT_FILE", envVar, 1) != 0)
			fatal("setenv");
	}
#ifdef __APPLE__
	envVar = getenv("NIX_COREFOUNDATION_RPATH");
	if (envVar == NULL) {
		if (setenv("NIX_COREFOUNDATION_RPATH", NIX_COREFOUNDATION_RPATH, 1) != 0)
			fatal("setenv");
	}
	envVar = getenv("PATH_LOCALE");
	if (envVar == NULL) {
		if (setenv("PATH_LOCALE", PATH_LOCALE, 1) != 0)
			fatal("setenv");
	}
#else  /* __APPLE__ */
	envVar = getenv("LOCALE_ARCHIVE");
	if (envVar == NULL) {
		if (setenv("LOCALE_ARCHIVE", LOCALE_ARCHIVE, 1) != 0)
			fatal("setenv");
	}
#endif /* __APPLE__ */

	/*
	 * Run the command.
	 */
	execvp(FLOXSH, argv);
	fatal("%s", FLOXSH);
}
