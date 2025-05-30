.DEFAULT_GOAL := all

ALL = env-filter flox-build.mk build-manifest.nix nef/default.nix \
  $(addprefix nef/lib/,default.nix dirToAttrs.nix extendAttrSet.nix mkOverlay.nix reflect.nix)

OS := $(shell uname -s)
ifeq (Linux,$(OS))
  ALL += libsandbox.so
else
  ALL += libsandbox.dylib
endif

closure.o: closure.c closure.h
	$(CC) -fPIC -c $<

libsandbox.so: sandbox.c closure.o
	$(CC) -shared -fPIC $^ -o $@
	patchelf --remove-rpath $@

libsandbox.dylib: sandbox.c closure.o
	$(CC) -pthread -dynamiclib $^ -o $@

env-filter: env-filter.bash
	cp $< $@
	chmod +x $@

.PHONY: all
all: $(ALL)

%/libexec_install: %
	@mkdir -p $(PREFIX)/libexec/$(dir $(@D))
	cp $< $(PREFIX)/libexec/$(@D)

.PHONY: install
install: $(addsuffix /libexec_install,$(ALL))

.PHONY: tests
tests:
	@echo TODO: create sandbox tests
