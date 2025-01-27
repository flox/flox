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
 * As with the parsing of FLOX_ENV_DIRS, it is essential that this parsing
 * of the closure be performant and initialized only once per invocation, so we
 * start by reading closure paths into a btable from $FLOX_ENV/requisites.txt.
 */

#define _GNU_SOURCE
#include <limits.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

// Declare version bindings to work with minimum supported GLIBC versions.
#ifdef linux
#include "glibc-bindings.h"
#endif

#define HASH_MULTIPLIER 31
#define INITIAL_CAPACITY FLOX_ENV_CLOSURE_MAXENTRIES

// Define the maximum number of paths to be tracked in the FLOX_ENV closure.
// This is somewhat arbitrary but should be more than enough for most cases.
#define FLOX_ENV_CLOSURE_MAXENTRIES 4096

// Define the maximum length of a directory path in the FLOX_ENV_DIRS
// environment variable. This is also somewhat arbitrary, but it should
// be more than enough for most cases.
#define FLOX_ENV_REQUISITE_MAXLEN PATH_MAX

typedef struct {
  char key[FLOX_ENV_REQUISITE_MAXLEN];
  bool is_filled;
} hash_entry_t;

typedef struct {
  hash_entry_t entries[FLOX_ENV_CLOSURE_MAXENTRIES];
  size_t size;
  size_t capacity;
} hash_table_t;

hash_table_t *hash_table_init(size_t capacity);
int hash_table_store(hash_table_t *table, const char *key);
bool hash_table_lookup(hash_table_t *table, const char *key);

// Helper macros for printing debug, warnings, errors.
static int debug_closure = -1;
static int sandbox_warn_count = 0;
#define debug(format, ...)                                                     \
  if (debug_closure)                                                           \
  fprintf(stderr, "CLOSURE DEBUG[%d]: " format "\n", getpid(), __VA_ARGS__)

// Temporary path buffer for calculating realpath of hash table queries.
static char realpath_buf[PATH_MAX];

static size_t hash(const char *key, size_t capacity) {
  size_t hash_value = 0;
  while (*key) {
    hash_value = hash_value * HASH_MULTIPLIER + (unsigned char)(*key);
    key++;
  }
  return hash_value % capacity;
}

hash_table_t *hash_table_init(size_t capacity) {
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
    return -1; // Table is full
  }

  size_t index = hash(key, table->capacity);
  while (table->entries[index].is_filled &&
         strcmp(table->entries[index].key, key) != 0) {
    index = (index + 1) % table->capacity;
  }

  if (!table->entries[index].is_filled) {
    strncpy(table->entries[index].key, key, FLOX_ENV_REQUISITE_MAXLEN - 1);
    table->entries[index].key[FLOX_ENV_REQUISITE_MAXLEN - 1] =
        '\0'; // Ensure null termination
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
  const char *pkgend = strchr(key + 44, '/');
  if (pkgend == NULL)
    return false;

  static char pkgbuf[PATH_MAX];
  (void)snprintf(pkgbuf, (pkgend - key) + 1, "%s", key);

  debug("hash_table_lookup(%s), looking for %s in hashtable", key, pkgbuf);

  size_t index = hash(pkgbuf, table->capacity);
  while (table->entries[index].is_filled) {
    // With Nix we only have to look at the first 44
    // characters to know that we have a match. e.g.
    // "/nix/store/12345678901234567890123456789012-foobar-1.2.3":
    //  ^^^^^^^^^^ ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
    //      10    1              32                1
    debug("comparing %s to %s", table->entries[index].key, pkgbuf);
    if (strncmp(table->entries[index].key, pkgbuf, 44) == 0) {
      debug("%s is in the closure", key);
      return true;
    }
    index = (index + 1) % table->capacity;
  }
  debug("%s is not in the closure", key);
  return false;
}

bool in_closure(const char *path) {
  static hash_table_t *table = NULL;

  // Debug closure library with FLOX_DEBUG_CLOSURE=1.
  debug_closure = (getenv("FLOX_DEBUG_CLOSURE") != NULL);

  if (!table) {
    const char *env_path = getenv("FLOX_ENV");
    if (!env_path) {
      fprintf(stderr, "FLOX_ENV environment variable not set\n");
      return false;
    }

    char requisites_path[256];
    snprintf(requisites_path, sizeof(requisites_path), "%s/requisites.txt",
             env_path);

    FILE *file = fopen(requisites_path, "r");
    if (!file) {
      perror("Error opening requisites.txt");
      return false;
    }

    table = hash_table_init(INITIAL_CAPACITY);

    char line[FLOX_ENV_REQUISITE_MAXLEN];
    static int count = 0;
    while (fgets(line, sizeof(line), file)) {
      line[strcspn(line, "\n")] = '\0'; // Remove newline character
      if (hash_table_store(table, line) != 0) {
        fprintf(stderr, "Error: Hash table is full, cannot store more paths\n");
        break;
      }
      count++;
    }
    fclose(file);

    // Because this library will itself be loaded on account of its presence
    // in LD_PRELOAD, we should ensure that we don't trip over ourselves.
    if (hash_table_store(table, "@@out@@") != 0) {
      fprintf(stderr, "Error: Hash table is full, cannot store more paths\n");
    }
    count++;

    // There is one more "blessed" path to be added to the table which is
    // the path of the manifest-built package itself, and this comes to us
    // by way of the FLOX_MANIFEST_BUILD_OUT environment variable.
    const char *additional_path = getenv("FLOX_MANIFEST_BUILD_OUT");
    if (additional_path) {
      if (hash_table_store(table, additional_path) != 0) {
        fprintf(stderr, "Error: Hash table is full, cannot store more paths\n");
      }
      count++;
    }

    debug("loaded %d entries from requisites.txt", count);
  }

  if (realpath(path, realpath_buf) == NULL) {
    // Likely that path does not exist, so just return true
    // so that the real system call can return ENOENT.
    debug("%s not found, allowing sandbox access", path);
    return true;
  }

  return hash_table_lookup(table, realpath_buf);
}
