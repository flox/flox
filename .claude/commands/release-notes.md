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
