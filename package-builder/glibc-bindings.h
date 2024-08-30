#ifndef GLIBC_BINDINGS_H
#define GLIBC_BINDINGS_H

// Declare version bindings to work with minimum supported GLIBC versions.
//
// This file needs to be updated whenever we start using a new GLIBC function.
// The procedure to update the file is as follows:
//
//   make -C ld-floxlib libsandbox.so
//   nm -D ld-floxlib/libsandbox.so | \
//     sed 's/@GLIBC_.*/@GLIBC_MIN_VERSION/' | awk '/GLIBC/ {print $NF}' | \
//     awk -F@ '{printf("__asm__( \".symver %s,%s@\" %s );\n",$1,$1,$2)}' >>
//     ld-floxlib/glibc-bindings.h
//   vi ld-floxlib/glibc-bindings.h
//     /* remove previous bindings section, leaving newly-appended one */
//
#if defined(__aarch64__)
// aarch64 Linux only goes back to 2.17.
#define GLIBC_MIN_VERSION "GLIBC_2.17"
#define ALT_GLIBC_MIN_VERSION "GLIBC_2.17"
#define ALT_ALT_GLIBC_MIN_VERSION "GLIBC_2.17"
#elif defined(__x86_64__)
// x86_64 Linux goes back to 2.2.5.
#define GLIBC_MIN_VERSION "GLIBC_2.2.5"
#define ALT_GLIBC_MIN_VERSION "GLIBC_2.3.4"
#define ALT_ALT_GLIBC_MIN_VERSION "GLIBC_2.4"
#else
#error "Unsupported architecture"
#endif

__asm__(".symver __cxa_finalize,__cxa_finalize@" GLIBC_MIN_VERSION);
__asm__(".symver dlsym,dlsym@" GLIBC_MIN_VERSION);
__asm__(".symver __errno_location,__errno_location@" GLIBC_MIN_VERSION);
__asm__(".symver fclose,fclose@" GLIBC_MIN_VERSION);
__asm__(".symver fgets,fgets@" GLIBC_MIN_VERSION);
__asm__(".symver fopen,fopen@" GLIBC_MIN_VERSION);
__asm__(".symver __fprintf_chk,__fprintf_chk@" ALT_GLIBC_MIN_VERSION);
__asm__(".symver fwrite,fwrite@" GLIBC_MIN_VERSION);
__asm__(".symver getenv,getenv@" GLIBC_MIN_VERSION);
__asm__(".symver getpid,getpid@" GLIBC_MIN_VERSION);
__asm__(".symver perror,perror@" GLIBC_MIN_VERSION);
__asm__(".symver __realpath_chk,__realpath_chk@" ALT_ALT_GLIBC_MIN_VERSION);
__asm__(".symver __snprintf_chk,__snprintf_chk@" ALT_GLIBC_MIN_VERSION);
__asm__(".symver __stack_chk_fail,__stack_chk_fail@" ALT_ALT_GLIBC_MIN_VERSION);
__asm__(".symver __stack_chk_guard,__stack_chk_guard@" GLIBC_MIN_VERSION);
__asm__(".symver stderr,stderr@" GLIBC_MIN_VERSION);
__asm__(".symver strchr,strchr@" GLIBC_MIN_VERSION);
__asm__(".symver strcmp,strcmp@" GLIBC_MIN_VERSION);
__asm__(".symver strcspn,strcspn@" GLIBC_MIN_VERSION);
__asm__(".symver strlen,strlen@" GLIBC_MIN_VERSION);
__asm__(".symver strncmp,strncmp@" GLIBC_MIN_VERSION);
__asm__(".symver strncpy,strncpy@" GLIBC_MIN_VERSION);
__asm__(".symver strtok_r,strtok_r@" GLIBC_MIN_VERSION);

#endif // GLIBC_BINDINGS_H
