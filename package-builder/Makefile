.DEFAULT_GOAL := all

ALL = flox-build.mk build-manifest.nix

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

.PHONY: all
all: $(ALL)

.PHONY: install
install: $(ALL)
	mkdir -p $(PREFIX)/libexec
	cp $^ $(PREFIX)/libexec

.PHONY: tests
tests:
	@echo TODO: create sandbox tests
