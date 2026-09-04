# The repository, narrowed to what the Rust builds read: the cargo
# workspace, plus the VERSION file they compile in.
#
# An allowlist rather than a denylist, which had to be extended by hand
# every time the repository grew a directory, and invalidated every build
# until someone did.
#
# The other packages here build from their own subdirectory and stay out,
# so an edit to one of them does not rebuild the rest.
{ lib }:
let
  root = ./../..;

  members = (builtins.fromTOML (builtins.readFile "${toString root}/Cargo.toml")).workspace.members;

  # What cargo reads before it reaches a crate; the crates are `members`.
  workspaceFiles = [
    ".cargo"
    "Cargo.toml"
    "Cargo.lock"
    "VERSION"
  ];
in
lib.fileset.toSource {
  inherit root;
  fileset = lib.fileset.unions (map (p: root + "/${p}") (workspaceFiles ++ members));
}
