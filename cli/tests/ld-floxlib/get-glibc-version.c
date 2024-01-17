#include <gnu/libc-version.h>
#include <stdio.h>

int main() {
  printf("GNU C Library (glibc) version: %s\n", gnu_get_libc_version());
  return 0;
}
