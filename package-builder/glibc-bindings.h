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

__asm__(".symver closedir,closedir@" GLIBC_MIN_VERSION);
__asm__(".symver __cxa_finalize,__cxa_finalize@" GLIBC_MIN_VERSION);
__asm__(".symver dlsym,dlsym@" GLIBC_MIN_VERSION);
__asm__(".symver __errno_location,__errno_location@" GLIBC_MIN_VERSION);
__asm__(".symver exit,exit@" GLIBC_MIN_VERSION);
__asm__(".symver fclose,fclose@" GLIBC_MIN_VERSION);
__asm__(".symver fflush,fflush@" GLIBC_MIN_VERSION);
__asm__(".symver fgets,fgets@" GLIBC_MIN_VERSION);
__asm__(".symver fnmatch,fnmatch@" GLIBC_MIN_VERSION);
__asm__(".symver __fprintf_chk,__fprintf_chk@" ALT_GLIBC_MIN_VERSION);
__asm__(".symver fwrite,fwrite@" GLIBC_MIN_VERSION);
__asm__(".symver getenv,getenv@" GLIBC_MIN_VERSION);
__asm__(".symver getpid,getpid@" GLIBC_MIN_VERSION);
__asm__(".symver opendir,opendir@" GLIBC_MIN_VERSION);
__asm__(".symver perror,perror@" GLIBC_MIN_VERSION);
// pthread_once is the core of the thread-safety fix. On glibc < 2.34 it lives
// in libpthread.so.0 (versioned at the baseline GLIBC for each arch); on 2.34+
// it is a compat symbol in libc. Bind to the minimum so the resulting library
// does not silently require GLIBC_2.34. (Also pass -pthread when linking; see
// the Makefile.)
__asm__(".symver pthread_once,pthread_once@" GLIBC_MIN_VERSION);
// pthread_mutex_lock/unlock guard the warned-paths dedup set. Like
// pthread_once they live in libpthread.so.0 on glibc < 2.34 (versioned at the
// baseline for each arch) and are compat symbols in libc on 2.34+; bind to the
// minimum so the library does not silently require GLIBC_2.34.
__asm__(".symver pthread_mutex_lock,pthread_mutex_lock@" GLIBC_MIN_VERSION);
__asm__(".symver pthread_mutex_unlock,pthread_mutex_unlock@" GLIBC_MIN_VERSION);
__asm__(".symver __realpath_chk,__realpath_chk@" ALT_ALT_GLIBC_MIN_VERSION);
__asm__(".symver __snprintf_chk,__snprintf_chk@" ALT_GLIBC_MIN_VERSION);
__asm__(".symver __stack_chk_fail,__stack_chk_fail@" ALT_ALT_GLIBC_MIN_VERSION);
__asm__(".symver __stack_chk_guard,__stack_chk_guard@" GLIBC_MIN_VERSION);
__asm__(".symver stderr,stderr@" GLIBC_MIN_VERSION);
__asm__(".symver strchr,strchr@" GLIBC_MIN_VERSION);
__asm__(".symver strcmp,strcmp@" GLIBC_MIN_VERSION);
__asm__(".symver strcspn,strcspn@" GLIBC_MIN_VERSION);
__asm__(".symver strdup,strdup@" GLIBC_MIN_VERSION);
__asm__(".symver strlen,strlen@" GLIBC_MIN_VERSION);
__asm__(".symver strncmp,strncmp@" GLIBC_MIN_VERSION);
__asm__(".symver strncpy,strncpy@" GLIBC_MIN_VERSION);
__asm__(".symver strtok_r,strtok_r@" GLIBC_MIN_VERSION);
// syscall(SYS_gettid) is used for the debug thread id instead of the gettid()
// wrapper, which only exists from glibc 2.30; syscall() has existed since the
// baseline, so binding it here keeps the minimum glibc at the target.
__asm__(".symver syscall,syscall@" GLIBC_MIN_VERSION);

#endif // GLIBC_BINDINGS_H
