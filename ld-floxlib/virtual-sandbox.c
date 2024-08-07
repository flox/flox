/*
 * The Flox "virtual sandbox" warns or aborts when encountering an ELF access
 * from outside the closure of packages implied by $FLOX_ENV. In this regard
 * it can provide the same guarantees at an ELF level provided by the sandbox
 * itself, but at an _advisory_ level, so that developers are informed of
 * missing dependencies without actually breaking anything.
 *
 * The virtual sandbox is enabled with `FLOX_VIRTUAL_SANDBOX=(warn|enforce)`
 * set in the environment, and we do this when wrapping files in the bin
 * directory in the course of performing a manifest build.
 *
 * As with the parsing of FLOX_ENV_LIB_DIRS, it is essential that this parsing
 * of the closure be performant and initialized only once per invocation, so we
 * start by reading closure paths into a btable from $FLOX_ENV/requisites.txt.
 */

#include "virtual-sandbox.h"
#include <stdlib.h>
#include <string.h>
#include <stdio.h>
#include <unistd.h>

#define HASH_MULTIPLIER 31
#define INITIAL_CAPACITY FLOX_ENV_CLOSURE_MAXENTRIES

// Uncomment the following line for debugging.
// #define _debug(format, ...) fprintf(stderr, "DEBUG[%d]: " format "\n", getpid(), __VA_ARGS__)
#define _debug(format, ...) (void)0

static size_t hash(const char *key, size_t capacity) {
    size_t hash_value = 0;
    while (*key) {
        hash_value = hash_value * HASH_MULTIPLIER + (unsigned char)(*key);
        key++;
    }
    return hash_value % capacity;
}

hash_table_t* hash_table_init(size_t capacity) {
    static hash_table_t table;
    table.size = 0;
    table.capacity = capacity;
    for (size_t i = 0; i < capacity; i++) {
        table.entries[i].is_filled = false;
    }
    return &table;
}

int hash_table_store(hash_table_t *table, const char *key) {
    if (table->size >= table->capacity) {
        return -1;  // Table is full
    }

    size_t index = hash(key, table->capacity);
    while (table->entries[index].is_filled && strcmp(table->entries[index].key, key) != 0) {
        index = (index + 1) % table->capacity;
    }

    if (!table->entries[index].is_filled) {
        strncpy(table->entries[index].key, key, FLOX_ENV_REQUISITE_MAXLEN - 1);
        table->entries[index].key[FLOX_ENV_REQUISITE_MAXLEN - 1] = '\0';  // Ensure null termination
        table->entries[index].is_filled = true;
        table->size++;
    }
    return 0;
}

bool hash_table_lookup(hash_table_t *table, const char *key) {
    // look for the first '/' following the expected 44 characters
    // in a /nix/store path, e.g.
    //   /nix/store/12345678901234567890123456789012-foobar-1.2.3/bin/foo":
    //   ^^^^^^^^^^ ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
    const char * pkgend = strchr( key+44, '/' );
    if (pkgend == NULL)
        return false;

    static char pkgbuf[PATH_MAX];
    (void) snprintf( pkgbuf, (pkgend-key)+1, "%s", key );

    _debug("hash_table_lookup(%s), looking for %s in hashtable", key, pkgbuf);

    size_t index = hash(pkgbuf, table->capacity);
    while (table->entries[index].is_filled) {
        // With Nix we only have to look at the first 44
        // characters to know that we have a match. e.g.
        // "/nix/store/12345678901234567890123456789012-foobar-1.2.3":
        //  ^^^^^^^^^^ ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
        //      10    1              32                1
        _debug("comparing %s to %s", table->entries[index].key, pkgbuf);
        if (strncmp(table->entries[index].key, pkgbuf, 44) == 0)
            return true;
        index = (index + 1) % table->capacity;
    }
    return false;
}

bool in_closure(const char *path) {
    static hash_table_t *table = NULL;

    // The `/usr/bin/env` path is ubiquitous and hardcoded to an extent that
    // we are faced with the choice of forcing developers to replace it in
    // code, or simply let it be an allowed exception.
    //
    // Once requested by way of the la_version() call, we know that all
    // libraries requested by this PID are similarly linked from /usr/bin/env
    // so we can simply give all lookups a free pass.
    //
    // TODO: make this list of allowed exceptions configurable.
    if ( freepass ) {
        return true;
    } else if (
        strcmp(path, "/usr/bin/env") == 0 ||
        strcmp(path, "/bin/sh") == 0 ||
        strcmp(path, "/usr/bin/dash") == 0
    ) {
        freepass = true;
        return true;
    }

    if (!table) {
        const char *env_path = getenv("FLOX_ENV");
        if (!env_path) {
            fprintf(stderr, "FLOX_ENV environment variable not set\n");
            return false;
        }

        char requisites_path[256];
        snprintf(requisites_path, sizeof(requisites_path), "%s/requisites.txt", env_path);

        FILE *file = fopen(requisites_path, "r");
        if (!file) {
            perror("Error opening requisites.txt");
            return false;
        }

        table = hash_table_init(INITIAL_CAPACITY);

        char line[FLOX_ENV_REQUISITE_MAXLEN];
        static int count=0;
        while (fgets(line, sizeof(line), file)) {
            line[strcspn(line, "\n")] = '\0'; // Remove newline character
            if (hash_table_store(table, line) != 0) {
                fprintf(stderr, "Error: Hash table is full, cannot store more paths\n");
                break;
            }
            count++;
        }
        fclose(file);

        // There is one more "blessed" path to be added to the table which is
        // the path of the manifest-built package itself, and this comes to us
        // by way of the FLOX_MANIFEST_BUILD_OUT environment variable.
        const char *manifest_build_out = getenv("FLOX_MANIFEST_BUILD_OUT");
        if (manifest_build_out) {
            if (hash_table_store(table, manifest_build_out) != 0) {
                fprintf(stderr, "Error: Hash table is full, cannot store more paths\n");
            }
            count++;
        }

        _debug("loaded %d entries from requisites.txt", count);
    }

    return hash_table_lookup(table, path);
}
