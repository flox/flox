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
#include <pthread.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

// Declare version bindings to work with minimum supported GLIBC versions.
#ifdef linux
#include "glibc-bindings.h"
// fopen is called directly in this file; version it here rather than in the
// shared header, because sandbox.c *defines* fopen as an interceptor and a
// versioned definition requires a linker version script we don't have.
__asm__(".symver fopen,fopen@" GLIBC_MIN_VERSION);
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

// Whether to emit verbose debug tracing, controlled by FLOX_DEBUG_CLOSURE.
// Written exactly once (under closure_table_once, below) and only read
// afterwards, so it needs no further synchronization.
static int debug_closure = 0;

// Emit a debug line when tracing is enabled.
//
// Wrapped in `do { ... } while (0)` so the macro is a single statement and is
// safe to use as the body of an `if`/`else` without surprising the surrounding
// control flow (the previous bare-`if` form silently swallowed adjacent
// `else` branches).
#define debug(format, ...)                                                     \
  do {                                                                         \
    if (debug_closure)                                                         \
      fprintf(stderr, "CLOSURE DEBUG[%d]: " format "\n", getpid(),             \
              __VA_ARGS__);                                                    \
  } while (0)

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
  // We key on the Nix store-object prefix: "/nix/store/" + 32-char hash + '-',
  // which is exactly 44 characters. Anything shorter than that cannot be a
  // store path, so reject it up front. This also guards the `key + 44`
  // arithmetic below from running off the end of a short string.
  //   /nix/store/12345678901234567890123456789012-foobar-1.2.3/bin/foo
  //   ^^^^^^^^^^ ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ ^
  //     (11)                 (32)               (1) = 44
  if (strlen(key) < 44)
    return false;

  // Locate the first '/' *after* the store-object prefix; the substring up to
  // it (e.g. "/nix/store/<hash>-foobar-1.2.3") is the store object we look up.
  // A key with no such '/' is a bare store path with no component beneath it
  // and never matches a requisite.
  const char *pkgend = strchr(key + 44, '/');
  if (pkgend == NULL)
    return false;

  // Copy just the store-object portion into a thread-local stack buffer.
  // NOTE: this MUST be a local (not a shared `static`) buffer — concurrent
  // lookups from multiple threads would otherwise clobber each other's value
  // in the window between writing it here and reading it in the loop below.
  char pkgbuf[PATH_MAX];
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

// The parsed closure table together with its one-time initializer.
//
// Building the table reads requisites.txt plus a couple of environment
// variables, and must happen exactly once even when the very first lookups
// arrive concurrently on several threads. We drive that with pthread_once():
// `closure_table` is published by the initializer and only read afterwards,
// and pthread_once() supplies the happens-before ordering, so the lookup path
// itself needs no locking. (The previous code did an unsynchronized
// check-then-build on a shared `static` pointer, so racing first calls could
// each parse the file into the same table.)
static pthread_once_t closure_table_once = PTHREAD_ONCE_INIT;
static hash_table_t *closure_table = NULL; // stays NULL if init fails

// One-time initializer, invoked via pthread_once() from in_closure().
//
// On any failure it leaves closure_table == NULL and prints a diagnostic;
// in_closure() then treats a NULL table as "nothing is in the closure", so a
// setup failure causes accesses to be flagged rather than silently allowed.
static void closure_table_init(void) {
  // Latch the debug flag once, here, while still effectively single-threaded.
  debug_closure = (getenv("FLOX_DEBUG_CLOSURE") != NULL);

  const char *env_path = getenv("FLOX_ENV");
  if (!env_path) {
    fprintf(stderr, "FLOX_ENV environment variable not set\n");
    return;
  }

  char requisites_path[PATH_MAX];
  snprintf(requisites_path, sizeof(requisites_path), "%s/requisites.txt",
           env_path);

  FILE *file = fopen(requisites_path, "r");
  if (!file) {
    perror("Error opening requisites.txt");
    return;
  }

  hash_table_t *table = hash_table_init(INITIAL_CAPACITY);

  int count = 0;
  char line[FLOX_ENV_REQUISITE_MAXLEN];
  while (fgets(line, sizeof(line), file)) {
    line[strcspn(line, "\n")] = '\0'; // strip the trailing newline

    // Strip any trailing slash(es). hash_table_lookup() keys on the slash-free
    // store-object prefix, and the table is hashed by the *whole* stored
    // string, so a requisite written as ".../store-object/" would hash into a
    // different bucket and silently never match — making an in-closure access
    // look out-of-closure. Normalizing here keeps store and lookup symmetric.
    size_t len = strlen(line);
    while (len > 0 && line[len - 1] == '/')
      line[--len] = '\0';

    if (hash_table_store(table, line) != 0) {
      fprintf(stderr, "Error: Hash table is full, cannot store more paths\n");
      break;
    }
    count++;
  }
  fclose(file);

  // Because this library is itself loaded via LD_PRELOAD /
  // DYLD_INSERT_LIBRARIES, bless a sentinel so we never flag our own output.
  if (hash_table_store(table, "@@out@@") != 0)
    fprintf(stderr, "Error: Hash table is full, cannot store more paths\n");
  else
    count++;

  // The manifest-built package's own output path is also blessed; it reaches
  // us via FLOX_MANIFEST_BUILD_OUT.
  const char *additional_path = getenv("FLOX_MANIFEST_BUILD_OUT");
  if (additional_path) {
    if (hash_table_store(table, additional_path) != 0)
      fprintf(stderr, "Error: Hash table is full, cannot store more paths\n");
    else
      count++;
  }

  debug("loaded %d entries from requisites.txt", count);

  // Publish the fully built table last. Until this store completes, concurrent
  // callers observe NULL (via pthread_once ordering) and decline to match.
  closure_table = table;
}

bool in_closure(const char *path) {
  // Build the closure table exactly once, even under concurrent first calls.
  pthread_once(&closure_table_once, closure_table_init);
  if (closure_table == NULL)
    return false; // initialization failed and was already diagnosed

  // Resolve into a *local* (stack) buffer. This used to be a shared file-scope
  // `realpath_buf`, which meant two threads resolving different paths would
  // overwrite each other between this call and the hash lookup below — the
  // central data race this rewrite eliminates.
  char realpath_buf[PATH_MAX];
  if (realpath(path, realpath_buf) == NULL) {
    // Most likely the path does not exist. Allow it through so the real system
    // call can surface ENOENT itself rather than us masking it with EACCES.
    debug("%s not found, allowing sandbox access", path);
    return true;
  }

  return hash_table_lookup(closure_table, realpath_buf);
}
