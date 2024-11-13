# NixOS doesn't ship with any default cache file.
cat >&2 << EOF
ldconfig: Can't open cache file /etc/ld.so.cache
: No such file or directory
EOF
exit 1
