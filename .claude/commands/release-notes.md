# Release-notes command

Generate release notes for the Flox CLI

Review the commit history between the last tag and the previous tag of the form
v.X.Y.Z.  Find the tags and confirm the correct scope with the user.  Scan
commits and if the github MCP server is present, scan PR comments included as
well.

Look for anything called out specifically as "Release Notes", breaking changes,
new features, fixed bugs and anything that could be user facing.

Generate 3 lists of items, one of things that were called out as "Release Notes"
or otherwise is clear it is something to be included.  Another of things that
you thing should be included but are questionable, and a final list of things
that could be included, but are more unsure.

For each list, number them for reference, and order them features first,
followed by changes, and lastly fixes.

Check with the user on what items to include, allowing them to ask for details
for any item, before compiling a final list that is a user facing summary of the
issues the user would like to include.


Phrase the items using the following guidelines:
- Group into "features" and "fixes".  Add a third group for "changes" only when
  an item is very clearly a change in behavior the user needs to be aware of.
- Frame all items in present tense as a single complete sentence, only adding a
  second sentence when needed for clarity or additional explanation.
- Use backticks "`" for command examples, environment variables or arguments.
- Note any commits that were made by community members (not flox.dev).

Use the following markdown template EXACTLY for formatting (updating the version in the
link accordingly. Use the exact formatting and markdown annotations, updating
only the list of features, fixes, community contributions (or removing it if
there are none), and the version in the links.

```
## Features
- Running command x will now do something cool

## Fixes
- Some command now properly does that.
- Using xyz will always do this.

## Thank you to our community contributions this release
- Some fix (@username)

## Download Links

* [DEB (x86_64-linux)](https://downloads.flox.dev/by-env/stable/deb/flox-1.0.0.x86_64-linux.deb)
* [DEB (aarch64-linux)](https://downloads.flox.dev/by-env/stable/deb/flox-1.0.0.aarch64-linux.deb)
* [RPM (x86_64-linux)](https://downloads.flox.dev/by-env/stable/rpm/flox-1.0.0.x86_64-linux.rpm)
* [RPM (aarch64-linux)](https://downloads.flox.dev/by-env/stable/rpm/flox-1.0.0.aarch64-linux.rpm)
* [OSX (x86_64-darwin)](https://downloads.flox.dev/by-env/stable/osx/flox-1.0.0.x86_64-darwin.pkg)
* [OSX (aarch64-darwin)](https://downloads.flox.dev/by-env/stable/osx/flox-1.0.0.aarch64-darwin.pkg
```

Upon completion and acceptence, offer to write the contents to a file so the
user can copy from that, explaining that copying from the terminal will likely
show leading spaces that will affect the markdown.
