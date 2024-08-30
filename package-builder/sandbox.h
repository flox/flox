#ifndef SANDBOX_H
#define SANDBOX_H

#include <stdbool.h>

bool sandbox_check_argv0();
bool sandbox_check_path(const char *path);
int get_sandbox_level();

#endif // SANDBOX_H
