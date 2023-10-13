# ============================================================================ #
#
# @brief Get a `rev` tag from a `sourceInfo` object if it is associated with the
#        `main` or `master` branches of a git repository.
#        Otherwise return `null`.
#
# @param sourceInfo A `sourceInfo` object, being the result of a
#                 `builtins.fetchTree` call.
#                 For flakes the `sourceInfo` object is available as `self` or
#                 the amalgamated `inputs` argument.
#
# @param gitDir Absolute path to the git repository's `.git` directory.
#               There is no way to access this from `sourceInfo` alone, so it
#               is expected to be a filesystem path outside of the `nix` store.
#
# @return A `revCount` or `shortRev` or `"dirty"` if @a sourceInfo is associated
#         with the `main` or `master` branches of a repository.
#         Otherwise `null`.
#
#
# ---------------------------------------------------------------------------- #
sourceInfo: gitDir: let
  # ---------------------------------------------------------------------------- #
  # @brief The revision tag of the source tree.
  # For `git` we will have `revCount`, for `github` we will have `shortRev`,
  # and for a dirty tree we will have neither.
  revTag =
    if sourceInfo ? revCount
    then "r" + (toString sourceInfo.revCount)
    else sourceInfo.shortRev or "dirty";

  # ---------------------------------------------------------------------------- #

  # @brief Parse a refspec from a file.
  # @param relPath Relative path to a file containing a refspec.
  #        For example `HEAD` or `refs/remotes/origin/HEAD`.
  # @return The refspec, or `null` if the file does not exist or does not
  #         contain a refspec.
  # To find `ref` we have to read the `HEAD` file in the `.git` directory.
  readRef = relPath: let
    headFile = gitDir + ("/" + relPath);
    headContents = builtins.readFile headFile;
    refMatch = builtins.match "ref: (refs/.+)\n" headContents;
  in
    if (builtins.pathExists headFile) && (refMatch != null)
    then builtins.head refMatch
    else null;

  # ---------------------------------------------------------------------------- #

  treeRef = readRef "HEAD";
  originRef = readRef "refs/remotes/origin/HEAD";

  treeRev =
    if treeRef == null
    then null
    else builtins.readFile (gitDir + "/" + treeRef);
  originRev =
    if originRef == null
    then null
    else builtins.readFile (gitDir + "/" + originRef);
  # ---------------------------------------------------------------------------- #
in
  if (revTag != null) && (treeRev == originRev)
  then revTag
  else null
# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #

