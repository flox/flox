.DEFAULT_GOAL = ld-floxlib.so
GREENCHECK = ✅
REDCROSS = ❌
OS := $(shell uname -s)

.DEFAULT_GOAL := all

ifeq (Linux,$(OS))
  ALL = libsandbox.so ld-floxlib.so
  TESTS = sandbox-tests ld-audit-tests
  IS_LINUX := true
else
  ALL = libsandbox.dylib
  TESTS = sandbox-tests
endif

# Define LD_FLOXLIB with macro to allow testing builds from other hosts.
LD_FLOXLIB ?= $(abspath ld-floxlib.so)

ld-floxlib.so: ld-floxlib.c
	$(CC) -shared -fPIC $< -o $@
	@# The ld-floxlib.so library only requires libc, which is guaranteed
	@# to either be already loaded or available by way of a default provided
	@# by the linker itself, so to avoid loading a different libc than the
	@# one already loaded we remove RPATH/RUNPATH from the shared library.
	patchelf --remove-rpath $@

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
	mkdir -p $(PREFIX)/lib
	cp $^ $(PREFIX)/lib

# Build unit-test linked with ld-floxlib.so.
unit-test: unit-test.c ld-floxlib.so
	$(CC) $< -o $@ ./ld-floxlib.so

# Set up test libraries.
tests/test1/libtest1.so: libtest.c
	mkdir -p $(@D)
	$(CC) -DTEST_FUNCTION=test1 -shared -fPIC $< -o $@

tests/test2/libtest2.so: libtest.c
	mkdir -p $(@D)
	$(CC) -DTEST_FUNCTION=test2 -shared -fPIC $< -o $@

tests/test3/libtest3.so: libtest.c
	mkdir -p $(@D)
	$(CC) -DTEST_FUNCTION=test3 -shared -fPIC $< -o $@
	# Render test3 directory unreadable
	chmod ugo-rwx $(@D)

.PHONY: testlibs
testlibs: tests/test1/libtest1.so tests/test2/libtest2.so tests/test3/libtest3.so

# By design ld-floxlib only parses FLOX_ENV_LIB_DIRS once per invocation,
# so testing la_objsearch() for various combinations of env and arg input
# requires multiple invocations. We use the following template as a way
# to generate targets for each test to be performed, while building a list
# of all targets in $(unit_tests).
define unit_test_TEMPLATE =
  $(eval _args = $(1))
  # name of unit test
  $(eval _test_name = $(word 1,$(_args)))
  # value of FLOX_ENV_LIB_DIRS
  $(eval _value = $(word 2,$(_args)))
  # "name" arg to pass to la_obsearch()
  $(eval _name = $(word 3,$(_args)))
  # expected return value
  $(eval _expected = $(word 4,$(_args)))
  # capture remaining args to recurse
  $(eval _remaining_args = $(wordlist 5,$(words $(_args)),$(_args)))

  .PHONY: _unit_test_$(_test_name)
  _unit_test_$(_test_name): unit-test testlibs
	LD_FLOXLIB_AUDIT=1 FLOX_ENV_LIB_DIRS=$(_value) \
	  ./$$< "$(_name)" "$(_expected)"
	@echo -e "$(GREENCHECK)" "$(_test_name)" 1>&2

  unit_tests += _unit_test_$(_test_name)

  # recurse if any remaining args
  $(if $(_remaining_args),$(call unit_test_TEMPLATE,$(_remaining_args)))
endef

# Declare unit test targets to be performed.
# test_name     FLOX_ENV_LIB_DIRS                     name_arg    expected_value
# ------------- ------------------------------------- ----------- --------------
$(eval $(call unit_test_TEMPLATE,$(strip \
  empty         ""                                    libtest1.so libtest1.so \
  no_exist      /does/not/exist                       libtest1.so libtest1.so \
  relative      tests/test1                           libtest1.so tests/test1/libtest1.so \
  absolute      $(abspath tests/test1)                libtest1.so $(abspath tests/test1)/libtest1.so \
  multiple      tests/test1:tests/test2               libtest1.so tests/test1/libtest1.so \
  unreadable    tests/test1:tests/test2:tests/test3   libtest3.so libtest3.so \
  empty_element tests/test1::tests/test2::tests/test3 libtest2.so tests/test2/libtest2.so \
)))

# Also configure integration tests which exercise ld-floxlib.so by
# way of rtld-audit(7).

define integration_test_TEMPLATE =
  $(eval _args = $(1))
  # name of unit test
  $(eval _test_name = $(word 1,$(_args)))
  # expected return code of test
  $(eval _rc = $(word 2,$(_args)))
  # capture remaining args to recurse
  $(eval _remaining_args = $(wordlist 3,$(words $(_args)),$(_args)))

  ifeq ($(_rc),0)
    integration_$(_test_name): integration-test.c $(unit_tests)
	$(CC) -DTEST_FUNCTION=$(_test_name) $$< -o $$@ \
	  -L./tests/$(_test_name) -l$(_test_name)

    run_integration_$(_test_name): integration_$(_test_name) ld-floxlib.so
	LD_FLOXLIB_AUDIT=1 LD_AUDIT=./ld-floxlib.so \
	  FLOX_ENV_LIB_DIRS=tests/test1:tests/test2:tests/test3 ./$$<
	@echo -e "$(GREENCHECK)" "$$@" 1>&2

    integration_tests += run_integration_$(_test_name)
  else
    integration_$(_test_name): integration-test.c $(unit_tests)
	-$(CC) -DTEST_FUNCTION=$(_test_name) $$< -o $$@ \
	  -L./tests/$(_test_name) -l$(_test_name)
	# The above compilation should fail, so if the target
	# exists then this test has failed.
	@if [ -f $$@ ]; then \
	  echo -e "$(REDCROSS)" "$$@" 1>&2; \
	  false; \
	else \
	  echo -e "$(GREENCHECK)" "$$@" 1>&2; \
	fi

    integration_tests += integration_$(_test_name)
  endif

  # recurse if any remaining args
  $(if $(_remaining_args),$(call integration_test_TEMPLATE,$(_remaining_args)))
endef

# Declare integration test targets to be performed.
# test_name return_code
# --------- -----------
$(eval $(call integration_test_TEMPLATE,$(strip \
  test1     0 \
  test2     0 \
  test3     1 \
)))

.PHONY: ld-audit-tests
ld-audit-tests: $(unit_tests) $(integration_tests)

.PHONY: sandbox-tests
sandbox-tests:
	@echo TODO: create sandbox tests

.PHONY: tests
tests: $(TESTS)
