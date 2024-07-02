.DEFAULT_GOAL = ld-floxlib.so

# Define LD_FLOXLIB with macro to allow testing builds from other hosts.
LD_FLOXLIB ?= $(abspath ld-floxlib.so)

ld-floxlib.so: ld-floxlib.c
	$(CC) -shared -fPIC $< -o $@
	# The ld-floxlib.so library only requires libc, which is guaranteed
	# to either be already loaded or available by way of a default provided
	# by the linker itself, so to avoid loading a different libc than the
	# one already loaded we remove RPATH/RUNPATH from the shared library.
	patchelf --remove-rpath $@

TESTLIBS = \
	libtest1.so \
	libtest2.so \
	libtest3.so

define test_TEMPLATE =
  # Test libs in subdirectories
  testlibdir$(n)/libtest$(n).so: libtest.c
	mkdir -p $$(@D)
	$(CC) -DTEST_FUNCTION=test$(n) -shared -fPIC $$< -o $$@

  test$(n): test.c testlibdir$(n)/libtest$(n).so
	$(CC) -DTEST_FUNCTION=test$(n) $$< -o $$@ -L./testlibdir$(n) -ltest$(n)
endef

$(foreach n,1 2 3,$(eval $(call test_TEMPLATE)))

FLOX_ENV_LIB_DIRS = testlibdir1:$(abspath testlibdir2):testlibdir3

.PHONY: test
test: $(LD_FLOXLIB) test1 test2 test3
	LD_FLOXLIB_DEBUG=1 LD_AUDIT=$< FLOX_ENV_LIB_DIRS=$(FLOX_ENV_LIB_DIRS) ./test1
	LD_FLOXLIB_DEBUG=1 LD_AUDIT=$< FLOX_ENV_LIB_DIRS=$(FLOX_ENV_LIB_DIRS) ./test2
	LD_FLOXLIB_DEBUG=1 LD_AUDIT=$< FLOX_ENV_LIB_DIRS=$(FLOX_ENV_LIB_DIRS) ./test3

.PHONY: install
install: ld-floxlib.so
	mkdir -p $(PREFIX)/lib
	cp $< $(PREFIX)/lib
