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

// atoi parses the optional :port and /cidr fields of FLOX_SANDBOX_ALLOW_NET
// entries. At the baseline GLIBC for each arch.
__asm__(".symver atoi,atoi@" GLIBC_MIN_VERSION);
// close() releases the per-request ask-broker socket fd (the RPC opens a
// fresh AF_UNIX connection per verdict and never caches the fd). At the
// baseline GLIBC for each arch.
__asm__(".symver close,close@" GLIBC_MIN_VERSION);
__asm__(".symver closedir,closedir@" GLIBC_MIN_VERSION);
__asm__(".symver __cxa_finalize,__cxa_finalize@" GLIBC_MIN_VERSION);
__asm__(".symver dlsym,dlsym@" GLIBC_MIN_VERSION);
__asm__(".symver __errno_location,__errno_location@" GLIBC_MIN_VERSION);
__asm__(".symver exit,exit@" GLIBC_MIN_VERSION);
__asm__(".symver fclose,fclose@" GLIBC_MIN_VERSION);
// fdopendir backs the directory-enumeration interceptor. It arrived in glibc
// 2.4 on x86_64 (and is at the 2.17 baseline on aarch64), so like
// __realpath_chk it pins at ALT_ALT — a version the library already
// references, so this adds no new floor. opendir/closedir are pinned at the
// baseline below, unchanged.
__asm__(".symver fdopendir,fdopendir@" ALT_ALT_GLIBC_MIN_VERSION);
__asm__(".symver fflush,fflush@" GLIBC_MIN_VERSION);
__asm__(".symver fgets,fgets@" GLIBC_MIN_VERSION);
__asm__(".symver fnmatch,fnmatch@" GLIBC_MIN_VERSION);
__asm__(".symver __fprintf_chk,__fprintf_chk@" ALT_GLIBC_MIN_VERSION);
__asm__(".symver fwrite,fwrite@" GLIBC_MIN_VERSION);
__asm__(".symver getenv,getenv@" GLIBC_MIN_VERSION);
__asm__(".symver getpid,getpid@" GLIBC_MIN_VERSION);
// inet_pton parses numeric IPv4/IPv6 allow-net entries; memcmp/memcpy/memset
// move and compare the raw address bytes for CIDR matching and entry building.
// All at the baseline GLIBC for each arch. (memcpy/memset are NOT compiled to
// __memcpy_chk/__memset_chk here because this library is built without
// _FORTIFY_SOURCE; pinning the plain symbols keeps the floor stable.)
__asm__(".symver inet_pton,inet_pton@" GLIBC_MIN_VERSION);
__asm__(".symver memcmp,memcmp@" GLIBC_MIN_VERSION);
__asm__(".symver memcpy,memcpy@" GLIBC_MIN_VERSION);
__asm__(".symver memset,memset@" GLIBC_MIN_VERSION);
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
// Sockets API for the broker RPC clients (the activation ask flow and the
// interactive prompt client). All have existed since the baseline GLIBC for
// each arch (2.17 on aarch64, 2.2.5 on x86_64), so binding them to
// GLIBC_MIN_VERSION matches the rest. read/write back the prompt client's
// line-protocol I/O; recv/send back the ask RPC.
__asm__(".symver connect,connect@" GLIBC_MIN_VERSION);
__asm__(".symver poll,poll@" GLIBC_MIN_VERSION);
__asm__(".symver read,read@" GLIBC_MIN_VERSION);
__asm__(".symver recv,recv@" GLIBC_MIN_VERSION);
__asm__(".symver send,send@" GLIBC_MIN_VERSION);
__asm__(".symver socket,socket@" GLIBC_MIN_VERSION);
// Network-egress mediation. The connect() interceptor formats the destination
// address for policy matching and messages: getaddrinfo()/freeaddrinfo()
// populate the IP->hostname attribution cache, and inet_ntop() stringifies the
// IPv4/IPv6 address for the warn/error lines and policy compare. inet_ntop is
// preferred over newer helpers because it has existed since the baseline GLIBC
// and does not raise the floor. The destination port is byte-swapped inline
// (a literal >> 8 / & 0xff on the network-order u16) rather than via ntohs(),
// which glibc resolves to an inline byte swap with no external symbol — so it
// needs no binding and pinning it would reference a symbol that may not exist.
__asm__(".symver getaddrinfo,getaddrinfo@" GLIBC_MIN_VERSION);
__asm__(".symver freeaddrinfo,freeaddrinfo@" GLIBC_MIN_VERSION);
__asm__(".symver inet_ntop,inet_ntop@" GLIBC_MIN_VERSION);
__asm__(".symver __realpath_chk,__realpath_chk@" ALT_ALT_GLIBC_MIN_VERSION);
__asm__(".symver __snprintf_chk,__snprintf_chk@" ALT_GLIBC_MIN_VERSION);
__asm__(".symver __stack_chk_fail,__stack_chk_fail@" ALT_ALT_GLIBC_MIN_VERSION);
__asm__(".symver __stack_chk_guard,__stack_chk_guard@" GLIBC_MIN_VERSION);
__asm__(".symver stderr,stderr@" GLIBC_MIN_VERSION);
// strcasecmp compares a cached hostname against a hostname allow-net entry
// case-insensitively. At the baseline GLIBC for each arch.
__asm__(".symver strcasecmp,strcasecmp@" GLIBC_MIN_VERSION);
__asm__(".symver strchr,strchr@" GLIBC_MIN_VERSION);
__asm__(".symver strcmp,strcmp@" GLIBC_MIN_VERSION);
__asm__(".symver strcspn,strcspn@" GLIBC_MIN_VERSION);
__asm__(".symver strdup,strdup@" GLIBC_MIN_VERSION);
__asm__(".symver strlen,strlen@" GLIBC_MIN_VERSION);
__asm__(".symver strncmp,strncmp@" GLIBC_MIN_VERSION);
__asm__(".symver strncpy,strncpy@" GLIBC_MIN_VERSION);
// strstr scans the ask broker's newline-JSON response for the verdict/scope/
// cache/req fields (a tolerant hand-rolled parser, no JSON library); strtoul
// parses the numeric req id out of that response. Both at the baseline GLIBC
// for each arch.
__asm__(".symver strstr,strstr@" GLIBC_MIN_VERSION);
__asm__(".symver strtok_r,strtok_r@" GLIBC_MIN_VERSION);
__asm__(".symver strtoul,strtoul@" GLIBC_MIN_VERSION);
// syscall(SYS_gettid) is used for the debug thread id instead of the gettid()
// wrapper, which only exists from glibc 2.30; syscall() has existed since the
// baseline, so binding it here keeps the minimum glibc at the target.
__asm__(".symver syscall,syscall@" GLIBC_MIN_VERSION);
// time() stamps the short deny-cache TTL under the ask flow. At the baseline
// GLIBC for each arch (chosen over clock_gettime, which was in librt at the
// x86_64 baseline and would raise the floor).
__asm__(".symver time,time@" GLIBC_MIN_VERSION);
// write() performs the single-shot O_APPEND record append in the audit-store
// hook (audit_append) and the prompt client's request write. At the baseline
// GLIBC for each arch.
__asm__(".symver write,write@" GLIBC_MIN_VERSION);

#endif // GLIBC_BINDINGS_H
