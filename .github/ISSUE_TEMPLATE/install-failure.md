---
name: Install failure
about: Sometimes things aren't working. Help us identify those things so we can fix them!
labels: bug

---

**Describe the bug:**

**Installer logs**

If on Linux, attach logs from `/tmp/flox-installation.log.$timestamp`, redacting anything sensitive.

If on macOS, attach logs for the install from `/var/log/install.log`. That should include everything from the first to the last mention of `com.floxdev.flox`, redacting anything sensitive.

```
awk '
/com\.floxdev\.flox/ {
  if (first == 0) first = NR
  last = NR
}
{ lines[NR] = $0 }
END {
  if (first && last && first <= last) {
    for (i = first; i <= last; i++) print lines[i]
  }
}' /var/log/install.log
```

**Flox Version (run `flox --version` if possible):**

**`uname -a` output:**
