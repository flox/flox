.DEFAULT_GOAL := all

ALL = validate-build flox-build.mk build-manifest.nix nef/default.nix \
  $(addprefix nef/lib/,default.nix dirToAttrs.nix extendAttrSet.nix instantiate.nix mkOverlay.nix reflect.nix)

OS := $(shell uname -s)
ifeq (Linux,$(OS))
  ALL += libsandbox.so
  SANDBOX_LIB := libsandbox.so
else
  ALL += libsandbox.dylib
  SANDBOX_LIB := libsandbox.dylib
endif

# -pthread is required because both sandbox.c and closure.c use pthread_once();
# on glibc < 2.34 pthread_once lives in libpthread.so.0, so it must be linked.
closure.o: closure.c closure.h
	$(CC) -pthread -fPIC -c $<

libsandbox.so: sandbox.c closure.o
	$(CC) -pthread -shared -fPIC $^ -o $@
	patchelf --remove-rpath $@

libsandbox.dylib: sandbox.c closure.o
	$(CC) -pthread -dynamiclib $^ -o $@

validate-build: validate-build.bash
	shellcheck $<
	cp $< $@
	chmod +x $@

.PHONY: all
all: $(ALL)

# Remove the compiled sandbox libraries and test binaries (the .gitignore'd
# build artifacts). Source is untouched. Lets `make clean tests` start from a
# pristine state so a stale object can never mask a source change.
.PHONY: clean
clean:
	rm -f closure.o libsandbox.so libsandbox.dylib \
	  tests/threadtest tests/sandbox_probe

%/libexec_install: %
	@mkdir -p $(PREFIX)/libexec/$(dir $(@D))
	cp $< $(PREFIX)/libexec/$(@D)

.PHONY: install
install: $(addsuffix /libexec_install,$(ALL))

# Test binaries.
#
# threadtest links the closure policy directly (no interception) and drives
# in_closure() from many threads to catch data races via a behavioral oracle.
tests/threadtest: tests/threadtest.c closure.c closure.h
	$(CC) -pthread -O2 -Wall -o $@ tests/threadtest.c closure.c

# sandbox_probe is a standalone "build process" stand-in. It is launched with
# the sandbox library injected (LD_PRELOAD / DYLD_INSERT_LIBRARIES) so its
# open()/openat() calls exercise the real interceptors.
tests/sandbox_probe: tests/sandbox_probe.c
	$(CC) -pthread -O2 -Wall -o $@ $<

# Run the full sandbox test suite (thread-safety oracle + interception
# semantics + threaded interception storm). Depends on the injected library and
# both test binaries.
.PHONY: tests
tests: $(SANDBOX_LIB) tests/threadtest tests/sandbox_probe
	bash tests/run-tests.sh
