ld-floxlib.so: ld-floxlib.c
	$(CC) -shared -fPIC $< -o $@
	# The ld-floxlib.so library only requires libc, which is guaranteed
	# to either be already loaded or available by way of a default provided
	# by the linker itself, so to avoid loading a different libc than the
	# one already loaded we remove RPATH/RUNPATH from the shared library.
	patchelf --remove-rpath $@

install: ld-floxlib.so
	mkdir -p $(PREFIX)/lib
	cp $< $(PREFIX)/lib
