/*
 * ld-floxlib - ld.so hack allowing Nix binaries to impurely
 *              load RHEL system libraries as last resort
 */

#ifndef _GNU_SOURCE
#define _GNU_SOURCE
#endif  /* _GNU_SOURCE */

#include <stdlib.h>
#include <unistd.h>
#include <sys/types.h>
#include <sys/param.h>
#include <limits.h>
#include <stdio.h>
#include <string.h>
#include <link.h>
#include <sys/stat.h>

static int audit_impure = -1;
static int debug_ld_floxlib = -1;
static char name_buf[PATH_MAX];

unsigned int
la_version(unsigned int version)
{
    return version;
}

char *
la_objsearch(const char *name, uintptr_t *cookie, unsigned int flag)
{
    struct stat stat_buf;

    if (audit_impure < 0)
        audit_impure = (getenv("LD_FLOXLIB_AUDIT_IMPURE") != NULL);
    if (debug_ld_floxlib < 0)
        debug_ld_floxlib = (getenv("LD_FLOXLIB_DEBUG") != NULL);

    if (debug_ld_floxlib)
        fprintf(stderr, "DEBUG: la_objsearch: %s\n", name);

    if (flag == LA_SER_DEFAULT && stat(name, &stat_buf) != 0) {
        char *basename = strrchr(name, '/');
        char *flox_env = getenv("FLOX_ENV");

        if (basename != NULL)
            basename++;
        else
            basename = (char *) name;

        if (debug_ld_floxlib)
            fprintf(stderr, "DEBUG: looking for: %s\n", basename);

#ifdef LD_FLOXLIB_LIB
        // First attempt to find the lib in the LD_FLOXLIB_LIB
        // cache of common libraries.
        (void) snprintf(name_buf, sizeof(name_buf), "%s/%s", LD_FLOXLIB_LIB, basename);
        if (debug_ld_floxlib)
            fprintf(stderr, "DEBUG: checking: %s\n", name_buf);
        if (stat(name_buf, &stat_buf) == 0) {
            if (audit_impure)
                fprintf(stderr, "AUDIT: %s -> %s\n", name, name_buf);
            return name_buf;
        }
#endif

        // Finally look for the lib in $FLOX_ENV/lib.
        if (flox_env != NULL) {
            (void) snprintf(name_buf, sizeof(name_buf), "%s/lib/%s", flox_env, basename);
            if (debug_ld_floxlib)
                fprintf(stderr, "DEBUG: checking: %s\n", name_buf);
            if (stat(name_buf, &stat_buf) == 0) {
                if (audit_impure)
                    fprintf(stderr, "AUDIT: %s -> %s\n", name, name_buf);
                return name_buf;
            }
        }
    }

    return (char *) name;
}

/* vim: set et ts=4: */
