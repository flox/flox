#ifndef VIRTUAL_SANDBOX_H
#define VIRTUAL_SANDBOX_H

#include <limits.h>
#include <stddef.h>
#include <stdbool.h>

// Define the maximum number of paths to be tracked in the FLOX_ENV closure.
// This is somewhat arbitrary but should be more than enough for most cases.
#define FLOX_ENV_CLOSURE_MAXENTRIES 4096

// Define the maximum length of a directory path in the FLOX_ENV_LIB_DIRS
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

hash_table_t* hash_table_init(size_t capacity);
int hash_table_store(hash_table_t *table, const char *key);
bool hash_table_lookup(hash_table_t *table, const char *key);
bool in_closure(const char *path);

// Once set to true, in_closure() will always return true. We use this for
// programs like `/usr/bin/env` that are ubiquitous across Linux distributions
// and hardcoded throughout countless codebases.
static bool freepass = false;

#endif // VIRTUAL_SANDBOX_H

