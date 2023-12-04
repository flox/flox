# -*- mode: autoconf; -*-
# ============================================================================ #
#
# @file m4/m4sh-util.m4
#
# @brief Extensions to Autoconf's `m4sh' shell scripting macros.
#
# This file was expropriated from
# [github:aakropotkin/ak-core](https://github.com/aakropotkin/ak-core/blob/main/src/m4sugar/m4sh-util.m4).
#
#
# ---------------------------------------------------------------------------- #

#serial 1

# ---------------------------------------------------------------------------- #

# Diversions

# When defining new sets of diversions avoid overwriting those used by other
# Autotools suites.
# Diversions from M4sh range from 0 (BINSH) to 6 (M4SH-INIT), and finally
# BODY is set to 1000 ( to intentionally make it last ).
# Autoconf uses 10 - 300
# Autotest uses 100 - 500
# Libtool and Automake don't use any, instead they append existing
# Autoconf diversions.
#
# We will use diversions 600~700 in this suite of extensions, and
# these utility functions generally shoot for the tail end at 690-700

m4_define([_m4_divert(DEFAULTS)], 690)dnl
m4_define([_m4_divert(DEFAULTS-STASH)], 691)dnl
m4_define([_m4_divert(REQUIRE-DEPS)], 692)dnl
m4_define([_m4_divert(PRE-BODY)], 699)dnl
m4_define([_m4_divert(PRE-BODY-SPACING)], 999)dnl


# ---------------------------------------------------------------------------- #

m4_defun([_AS_UTIL_LASTWORD_PREPARE],
[AS_USE_PROGS([SED])dnl
AS_REQUIRE_SHELL_FN([as_fn_lastword],
  [AS_FUNCTION_DESCRIBE([as_fn_lastword], [],
    [Print the last space separated word read from STDIN.])],
  [AS_IF([AS_UTIL_HAS_PROG([${SED:?}])],
    [$SED 's/^.*@<:@@<:@:space:@:>@@:>@\(.*\)@S|@/\1/' </dev/stdin],
    [while read -d ' ' _as_lastword_val; do :; done </dev/stdin])
  AS_ECHO("$_as_lastword_val")
  AS_UNSET([_as_lastword_val])])dnl
])# _AS_UTIL_LASTWORD_PREPARE

# AS_LASTWORD([CMD])
# ------------------
# Print the last space separated word read from STDIN.
# If CMD is provided, execute it as a shell command to be piped to STDIN.
m4_defun([AS_LASTWORD],
[AS_REQUIRE([_AS_UTIL_LASTWORD_PREPARE])dnl
m4_ifval([$1], [{ $1; }|])as_fn_lastword[]dnl
])# AS_LASTWORD


# ---------------------------------------------------------------------------- #

m4_defun([_AS_UTIL_PROG_PATH_PREPARE],
[AS_REQUIRE_SHELL_FN([as_fn_prog_path],
  [AS_FUNCTION_DESCRIBE([as_fn_prog_path], [PROG],
    [Find absolute path to PROG by searching PATH. ]dnl
    [Return non-zero if missing.])],
  [local _as_util_prog_path
  _as_util_prog_path="$( command command -v ${1@%:@@%:@* -- }; )"
  AS_IF([test -n "$_as_util_prog_path"],
    [AS_ECHO(["$_as_util_prog_path"])
    return 0],
    [return 1])])dnl
])# _AS_UTIL_PROG_PATH_PREPARE

# AS_UTIL_PROG_PATH(PROG)
# -----------------------
m4_defun([AS_UTIL_PROG_PATH],
[AS_REQUIRE([_AS_UTIL_PROG_PATH_PREPARE])[]as_fn_prog_path "$1"[]dnl
])# AS_UTIL_PROG_PATH


# ---------------------------------------------------------------------------- #

m4_defun([_AS_UTIL_HAS_PROG_PREPARE],
[AS_REQUIRE_SHELL_FN([as_fn_has_prog],
  [AS_FUNCTION_DESCRIBE([as_fn_has_prog], [PROG],
    [Test if PROG is in PATH. Set status accordingly.])],
  [# In cases where commands are overridden for sudoing, splicing on '--' helps.
  test -n "$( command command -v "${1@%:@@%:@* -- }"; )"])
])# _AS_UTIL_HAS_PROG_PREPARE

# AS_UTIL_HAS_PROG(PROG)
# ----------------------
m4_defun([AS_UTIL_HAS_PROG],
[AS_REQUIRE([_AS_UTIL_HAS_PROG_PREPARE])[]as_fn_has_prog "$1"[]dnl
])# AS_UTIL_HAS_PROG


# ---------------------------------------------------------------------------- #

m4_define([AS_UTIL_DEFUN_PROG],
[m4_ifndef([AS_PROG_$1], [m4_defun([AS_PROG_$1],
  [m4_divert_once([DEFAULTS],
    [: "${$1:=$2[]m4_ifnblank([$3], [ $3])}"; export $1])])])dnl
])# AS_UTIL_DEFUN_PROG

# AS_UTIL_DEF_PROG(VAR, PROG, [ARGS])
# -----------------------------------
# Define a program to be used in this script.
# Create an abstracted variable VAR, with default value of PROG, allowing users
# to override this value.
# If provided, ARGS will be used in the default value.
m4_define([AS_UTIL_DEF_PROG], [AS_UTIL_DEFUN_PROG($@)AS_REQUIRE([AS_PROG_$1])])

AC_DEFUN([AC_DEF_PROG], [AS_UTIL_DEF_PROG($@)AC_SUBST([$1])])

m4_define([AS_USE_PROGS], [m4_map_args_sep([AS_REQUIRE(AS_PROG_], [)], [], $@)])


# ---------------------------------------------------------------------------- #

# AS_UTIL_DEFUN_REQUIRED_PROG(VAR, PROG, [ARGS])
# ----------------------------------------------
m4_define([AS_UTIL_DEFUN_REQUIRED_PROG],
[AS_UTIL_DEFUN_PROG($@)dnl
m4_ifndef([AS_REQUIRED_PROG_$1],
  [m4_defun([AS_REQUIRE_PROG_$1],
    [AS_REQUIRE([_AS_UTIL_PROG_PATH_PREPARE])dnl
    AS_REQUIRE([_AS_UTIL_HAS_PROG_PREPARE])dnl
    AS_REQUIRE([AS_PROG_$1])dnl
    m4_divert_text([REQUIRE-DEPS],
      [AS_IF([! as_fn_has_prog "$[]$1"],
        [AS_ERROR([Required program $1 ($2) could not be found.])])
])])])dnl
])# AS_UTIL_DEFUN_REQUIRED_PROG

# AS_UTIL_REQUIRE_PROG(VAR, PROG, [ARGS])
# ---------------------------------------
m4_define([AS_UTIL_REQUIRE_PROG],
[AS_UTIL_DEFUN_REQUIRED_PROG($@)AS_REQUIRE([AS_REQUIRE_PROG_$1])dnl
])# AS_UTIL_REQUIRE_PROG


# ---------------------------------------------------------------------------- #

# AS_UTIL_ASSERT_PROG(VAR, PROG, [ARGS])
# --------------------------------------
m4_define([AS_UTIL_ASSERT_PROG],
[m4_ifndef([AS_REQUIRE_PROG_$1],
[AS_UTIL_DEF_PROG([$1], [$2])dnl
AS_IF([AS_UTIL_HAS_PROG([$[]$1])], [],
      [AS_ERROR([Required program $1 ($2) could not be found.])])])dnl
])# AS_UTIL_ASSERT_PROG


# ---------------------------------------------------------------------------- #

m4_defun([_AS_UTIL_CLEANUP_PREPARE],
[AS_USE_PROGS([RM], [TEST])
@%:@ Cleanup hook, triggered by signal interrupts.
@%:@ Adding members to CLEANUP_FILES and CLEANUP_DIRS will ensure that they are
@%:@ deleted if this script is terminated, or exits.
@%:@ NOTE: CLEANUP_TRAP_SIGNALS is effectively readonly after `trap' is set.
: "${CLEANUP_TRAP_SIGNALS:=HUP EXIT INT TERM QUIT}"
: "${CLEANUP_FILES:=}"
: "${CLEANUP_DIRS:=}"
AS_REQUIRE_SHELL_FN([as_fn_trap_cleanup],
  [AS_FUNCTION_DESCRIBE([as_fn_trap_cleanup], [],
    [Hook executed if a trapped signal fires.])],
  [_as_util_status="$?"
  AS_FOR([_as_cleanup_file], [CLEANUP_FILE], [$CLEANUP_FILES],
         [${TEST:?} -e "_as_cleanup_file" && ${RM:?} -f "_as_cleanup_file"])
  AS_FOR([_as_cleanup_dir], [CLEANUP_DIR], [$CLEANUP_DIRS],
         [${TEST:?} -d "_as_cleanup_dir" && ${RM:?} -rf "_as_cleanup_dir"])
  AS_EXIT(["$_as_util_status"])])
trap -- 'as_fn_trap_cleanup' $CLEANUP_TRAP_SIGNALS
])# _AS_UTIL_CLEANUP_PREPARE


# ---------------------------------------------------------------------------- #

m4_defun([_AS_UTIL_MKTEMP_PREPARE],
[AS_REQUIRE([_AS_UTIL_CLEANUP_PREPARE])dnl
AS_UTIL_REQUIRE_PROG([MKTEMP], [mktemp])dnl
AS_REQUIRE_SHELL_FN([as_fn_mktemp],
  [AS_FUNCTION_DESCRIBE([as_fn_mktemp], [@<:@FILENAME@:>@],
    [Create a temporary file in TMPDIR which will be deleted on exit. ]dnl
    [Sets the shell variable tmp to the temporary filename, and written ]dnl
    [to STDOUT.])],
  [local _as_util_mktemp_status
  tmp="$( ${MKTEMP:?} "${TMPDIR:=/tmp}/${1:-$as_me}-XXXXXX"; )"
  _as_util_mktemp_status="$?"
  AS_IF([test "${_as_util_mktemp_status:?}" -ne 0],
    [AS_UNSET([tmp])
    return "$_as_util_mktemp_status"],
    [CLEANUP_FILES="${CLEANUP_FILES+$CLEANUP_FILES }$tmp"
    AS_ECHO(["$tmp"])])])dnl
])# _AS_UTIL_MKTEMP_PREPARE

# AS_MKTEMP([FILENAME-PREFIX = $as_me ])
# --------------------------------------
# Create a temporary file in TMPDIR which will be deleted on exit.
# Sets the shell variable tmp to the temporary filename, and written
# to STDOUT.])],
m4_define([AS_MKTEMP],
[AS_REQUIRE([_AS_UTIL_MKTEMP_PREPARE])as_fn_mktemp $1[]dnl
])# AS_MKTEMP


# ---------------------------------------------------------------------------- #

m4_defun([_AS_UTIL_MKTEMPDIR_PREPARE],
[AS_REQUIRE([_AS_UTIL_CLEANUP_PREPARE])dnl
AS_UTIL_REQUIRE_PROG([MKTEMP], [mktemp])dnl
AS_REQUIRE_SHELL_FN([as_fn_mktempdir],
  [AS_FUNCTION_DESCRIBE([as_fn_mktempdir], [@<:@DIRNAME@:>@],
    [Create a temporary sub-directory in TMPDIR which will be deleted ]dnl
    [on exit. Sets the shell variable tmp to the temporary directory name, ]dnl
    [and written to STDOUT.])],
  [local _as_util_mktemp_status
  tmp="$( ${MKTEMP:?} -d "${TMPDIR:=/tmp}/${1:-$as_me.d}-XXXXXX"; )"
  _as_util_mktemp_status="$?"
  AS_IF([test "${_as_util_mktemp_status:?}" -ne 0],
    [AS_UNSET([tmp])
    return "$_as_util_mktemp_status"],
    [CLEANUP_FILES="${CLEANUP_DIRS+$CLEANUP_DIRS }$tmp"
    AS_ECHO(["$tmp"])])])dnl
])# _AS_UTIL_MKTEMPDIR_PREPARE

# AS_MKTEMPDIR([DIRNAME-PREFIX = $as_me.d])
# -----------------------------------------
# Create a temporary sub-directory in TMPDIR which will be deleted
# on exit. Sets the shell variable tmp to the temporary directory name,
# and written to STDOUT.
m4_define([AS_MKTEMPDIR],
[AS_REQUIRE([_AS_UTIL_MKTEMPDIR_PREPARE])as_fn_mktempdir $1[]dnl
])# AS_MKTEMPDIR


# ---------------------------------------------------------------------------- #

m4_defun([_AS_UTIL_DLFILE_PREPARE],
[AS_UTIL_DEF_PROG([CURL], [curl])dnl
AS_UTIL_DEF_PROG([WGET], [wget])dnl
AS_USE_PROGS([TEST], [MV])dnl
AS_REQUIRE([_AS_UTIL_HAS_PROG_PREPARE])dnl
m4_divert_text([REQUIRE-DEPS],
[AS_IF([as_fn_has_prog "$WGET"],
      [: "${_as_util_dlfile_prog:=WGET}"],
      [AS_IF([as_fn_has_prog "$CURL"],
             [: "${_as_util_dlfile_prog:=CURL}"],
             [: "${_as_util_dlfile_prog:=MISSING}"])])])dnl
AS_REQUIRE_SHELL_FN([as_fn_dlfile],
  [AS_FUNCTION_DESCRIBE([as_fn_dlfile], [URI OUTFILE],
    [Download a file from URI, saving it to OUTFILE.])],
  [local _as_dlfile_URI _as_dlfile_OUTFILE _as_dlfile_ODIR _as_dlfile_status
  _as_dlfile_URI="${1?}"
  _as_dlfile_OUTFILE="${2?}"
  AS_IF([$TEST -e "$_as_dlfile_OUTFILE"],
        [$MV -f "$_as_dlfile_OUTFILE" "$_as_dlfile_OUTFILE~"])
  _as_dlfile_ODIR="$( AS_DIRNAME(["$_as_dlfile_OUTFILE"]); )"
  AS_MKDIR_P(["$_as_dlfile_ODIR"])
  AS_CASE(["$_as_util_dlfile_prog"],
    [CURL], [$CURL -s -L "$_as_dlfile_URI" -o "$_as_dlfile_OUTFILE"],
    [WGET], [$WGET -q "$_as_dlfile_URI" -O "$_as_dlfile_OUTFILE"],
    [MISSING],
      [AS_ERROR([Couldn't find curl or wget to fetch the install script.]
[I'll assume that a developer that that's rugged enough to survive without ]
[these utilities will manage to acquire these files using smoke signals or ]
[carrier pidgeons?]
[In either case, I stay warm out there cowboy.])],
    [AS_ERROR([Urecognized DLFILE prog: $_as_util_dlfile_prog])])
  _as_dlfile_status="$?"
  if test "$_as_dlfile_status" -ne 0; then
    AS_WARN([Failed to download file $_as_dlfile_OUTFILE from $_as_dlfile_URI.])
  fi
  return "$_as_dlfile_status"])dnl
])# _AS_UTIL_DLFILE_PREPARE

# AS_UTIL_DLFILE(URI, OUTFILE)
# ----------------------------
# Download file from URI, saving it to OUTFILE.
# Either CURL or WGET will be used.
# An error will be thrown if neither program is available.
# If OUTFILE exists, it will be backed up to "$OUTFILE~".
# Any missing directories required to save OUTFILE will be created upfront;
# this is performed regardless of whether or not downloading succeeds.
m4_define([AS_UTIL_DLFILE],
[AS_REQUIRE([_AS_UTIL_DLFILE_PREPARE])[]as_fn_dlfile "$1" "$2"[]dnl
])# AS_UTIL_DLFILE


# ---------------------------------------------------------------------------- #

m4_defun([_AS_UTIL_FAIL_IF_ROOT_PREPARE],
[AS_REQUIRE_SHELL_FN([as_fn_fail_if_root],
  [AS_FUNCTION_DESCRIBE([as_fn_fail_if_root], [],
    [Assert that this script is not being run by the root user.])],
  [AS_VAR_IF([USER], [root],
    [AS_ERROR([This script may not be run by the root user.])])])dnl
])# _AS_UTIL_FAIL_IF_ROOT_PREPARE

# AS_FAIL_IF_ROOT
# ---------------
# Throw an error if $USER = root.
m4_define([AS_FAIL_IF_ROOT],
[AS_REQUIRE([_AS_UTIL_FAIL_IF_ROOT_PREPARE])[]as_fn_fail_if_root[]dnl
])# AS_FAIL_IF_ROOT


# ---------------------------------------------------------------------------- #

m4_defun([_AS_UTIL_SUDO_EVAL_PREPARE],
[AS_UTIL_REQUIRE_PROG([SUDO], [sudo])dnl
m4_divert_text([M4SH-SANITIZE],
  [CONFIG_SHELL="$( command command -v bash; )"])dnl
AS_REQUIRE_SHELL_FN([as_fn_sudo_eval],
  [AS_FUNCTION_DESCRIBE([as_fn_sudo_eval], [USER COMMAND...],
    [Run command as user, passing exported env vars and functions.]
    [NOTE: Unexported variables are not passed.])],
  [local _as_util_sudo_eval_user
  _as_util_sudo_eval_user="$[]1"
  shift
  ${SUDO:?} -E -u "$_as_util_sudo_eval_user"]dnl
  [-- ${SHELL:?} -c "$( declare -f; ); $[]*;"])dnl
# Extra variables set by M4sh that should be passed for functions to work.
export as_unset as_nop as_nl as_expr as_basename as_me
])# _AS_UTIL_SUDO_EVAL_PREPARE


# AS_SUDO_EVAL([USER], COMMAND)
# -----------------------------
# Evaluate COMMAND as USER, passing any exported environment variables and
# defined functions.
# NOTE: Unexported variables are not passed.
m4_define([AS_SUDO_EVAL],
[AS_REQUIRE([_AS_UTIL_SUDO_EVAL_PREPARE])dnl
m4_case([$@%:@], [1], [as_fn_sudo_eval root $1], [as_fn_sudo_eval $1 $2])[]dnl
])# AS_SUDO_EVAL


# ---------------------------------------------------------------------------- #

m4_defun([_AS_UTIL_DELETE_MATCHING_LINES_PREPARE],
[AS_USE_PROGS([ED], [SED], [GREP], [TEST], [RM])dnl
AS_REQUIRE_SHELL_FN([as_fn_delete_matching_lines],
  [AS_FUNCTION_DESCRIBE([as_fn_delete_matching_lines], [FILE PATTERN],
    [Use any of ED, SED to delete matching lines from FILE.]
    [Editing is performed inplace, but may require a temporary file.])],
  [AS_IF([AS_UTIL_HAS_PROG([$GREP]) && ! ${GREP:?} -q "$[]2" "$[]1"],
         [return 0])
  AS_IF([! ${TEST:?} -w $[]1], [AS_WARN([Cannot edit file: $[]1]); return 1])
  AS_IF([AS_UTIL_HAS_PROG([$ED])],
         [AS_ECHO(["g/$[]2/d${as_nl}wq"])|${ED:?} "$[]1"; return])
  AS_IF([AS_UTIL_HAS_PROG([$SED])],
         [${SED:?} -i~ "/$[]2/d" "$[]1" && ${RM:?} -f "$[]1~"; return])
  AS_ERROR([Cannot find either ED or SED to edit file: $[]1])])
])# _AS_UTIL_DELETE_MATCHING_LINES_PREPARE

# AS_DELETE_MATCHING_LINES(FILE, PATTERN)
# ---------------------------------------
# Use any of ED, SED to delete matching lines from FILE.
# Editing is performed inplace, but may require a temporary file.
m4_define([AS_DELETE_MATCHING_LINES],
[AS_REQUIRE([_AS_UTIL_DELETE_MATCHING_LINES_PREPARE])[]dnl
as_fn_delete_matching_lines $1 "$2"[]dnl
])# AS_DELETE_MATCHING_LINES


# ---------------------------------------------------------------------------- #

m4_defun([_AS_UTIL_DELETE_MATCHING_RANGE_PREPARE],
[AS_USE_PROGS([ED], [SED], [GREP], [TEST], [RM])
AS_REQUIRE_SHELL_FN([as_fn_delete_matching_range],
  [AS_FUNCTION_DESCRIBE([as_fn_delete_range_lines], [FILE PATTERN1 PATTERN2],
    [Use any of ED, SED to delete between matching lines in FILE.]
    [Editing is performed inplace, but may require a temporary file.])],
  [AS_IF([AS_UTIL_HAS_PROG([${GREP:?}]) &&  \
    ! { $GREP -q "$[]2" "$[]1" && $GREP -q "$[]3" "$[]1"; }],
    [return 0])
  AS_IF([! ${TEST:?} -w $[]1], [AS_WARN([Cannot edit file: $[]1]); return 1])
  AS_IF([AS_UTIL_HAS_PROG([${ED:?}])],
         [AS_ECHO(["/$[]2/,/$[]3/d${as_nl}wq"])|$ED "$[]1"; return])
  AS_IF([AS_UTIL_HAS_PROG([${SED:?}])],
         [$SED -i~ "/$[]2/,/$[]3/d" "$[]1" && ${RM:?} -f "$[]1~"; return])
  AS_ERROR([Cannot find either ED or SED to edit file: $[]1])])
])# _AS_UTIL_DELETE_MATCHING_RANGE_PREPARE

# AS_DELETE_MATCHING_RANGE(FILE, PATTERN1, PATTERN2)
# --------------------------------------------------
# Use any of ED, SED to delete between matching lines in FILE.
# Editing is performed inplace, but may require a temporary file.
m4_define([AS_DELETE_MATCHING_RANGE],
[AS_REQUIRE([_AS_UTIL_DELETE_MATCHING_RANGE_PREPARE])[]dnl
as_fn_delete_matching_range $1 "$2" "$3"[]dnl
])# AS_DELETE_MATCHING_RANGE


# ---------------------------------------------------------------------------- #

m4_defun([AS_UTILS_COREUTILS_FULL_PREPARE],
[AS_REQUIRE([_AS_UTILS_COREUTILS_PREPARE])
AS_UTIL_DEFUN_PROG([B2SUM], [b2sum])dnl
AS_UTIL_DEFUN_PROG([BASE32], [base32])dnl
AS_UTIL_DEFUN_PROG([BASE64], [base64])dnl
AS_UTIL_DEFUN_PROG([BASENC], [basenc])dnl
AS_UTIL_DEFUN_PROG([CHCON], [chcon])dnl
AS_UTIL_DEFUN_PROG([CHROOT], [chroot])dnl
AS_UTIL_DEFUN_PROG([CSPLIT], [csplit])dnl
AS_UTIL_DEFUN_PROG([DD], [dd])dnl
AS_UTIL_DEFUN_PROG([DF], [df])dnl
AS_UTIL_DEFUN_PROG([DIR], [dir])dnl
AS_UTIL_DEFUN_PROG([DIRCOLORS], [dircolors])dnl
AS_UTIL_DEFUN_PROG([DU], [du])dnl
AS_UTIL_DEFUN_PROG([EXPAND], [expand])dnl
AS_UTIL_DEFUN_PROG([FACTOR], [factor])dnl
AS_UTIL_DEFUN_PROG([FMT], [fmt])dnl
AS_UTIL_DEFUN_PROG([FOLD], [fold])dnl
AS_UTIL_DEFUN_PROG([HOSTID], [hostid])dnl
AS_UTIL_DEFUN_PROG([ID], [id])dnl
AS_UTIL_DEFUN_PROG([JOIN], [join])dnl
AS_UTIL_DEFUN_PROG([LINK], [link])dnl
AS_UTIL_DEFUN_PROG([LOGNAME], [logname])dnl
AS_UTIL_DEFUN_PROG([MKFIFO], [mkfifo])dnl
AS_UTIL_DEFUN_PROG([MKNOD], [mknod])dnl
AS_UTIL_DEFUN_PROG([NL], [nl])dnl
AS_UTIL_DEFUN_PROG([NOHUP], [nohup])dnl
AS_UTIL_DEFUN_PROG([NPROC], [nproc])dnl
AS_UTIL_DEFUN_PROG([NUMFMT], [numfmt])dnl
AS_UTIL_DEFUN_PROG([OD], [od])dnl
AS_UTIL_DEFUN_PROG([PASTE], [paste])dnl
AS_UTIL_DEFUN_PROG([PATHCHK], [pathchk])dnl
AS_UTIL_DEFUN_PROG([PINKY], [pinky])dnl
AS_UTIL_DEFUN_PROG([PR], [pr])dnl
AS_UTIL_DEFUN_PROG([PTX], [ptx])dnl
AS_UTIL_DEFUN_PROG([RUNCON], [runcon])dnl
AS_UTIL_DEFUN_PROG([SHA1SUM], [sha1sum])dnl
AS_UTIL_DEFUN_PROG([SHA224SUM], [sha224sum])dnl
AS_UTIL_DEFUN_PROG([SHA384SUM], [sha384sum])dnl
AS_UTIL_DEFUN_PROG([SHA512SUM], [sha512sum])dnl
AS_UTIL_DEFUN_PROG([SHRED], [shred])dnl
AS_UTIL_DEFUN_PROG([SHUF], [shuf])dnl
AS_UTIL_DEFUN_PROG([SPLIT], [split])dnl
AS_UTIL_DEFUN_PROG([STDBUF], [stdbuf])dnl
AS_UTIL_DEFUN_PROG([STTY], [stty])dnl
AS_UTIL_DEFUN_PROG([SYNC], [sync])dnl
AS_UTIL_DEFUN_PROG([TAC], [tac])dnl
AS_UTIL_DEFUN_PROG([TIMEOUT], [timeout])dnl
AS_UTIL_DEFUN_PROG([TRUNCATE], [truncate])dnl
AS_UTIL_DEFUN_PROG([TSORT], [tsort])dnl
AS_UTIL_DEFUN_PROG([TTY], [tty])dnl
AS_UTIL_DEFUN_PROG([UNEXPAND], [unexpand])dnl
AS_UTIL_DEFUN_PROG([UNLINK], [unlink])dnl
AS_UTIL_DEFUN_PROG([UPTIME], [uptime])dnl
AS_UTIL_DEFUN_PROG([VDIR], [vdir])dnl
AS_UTIL_DEFUN_PROG([WHO], [who])dnl
])# AS_UTILS_COREUTILS_FULL_PREPARE

# Coreutils that we actually care about.
m4_defun([_AS_UTILS_COREUTILS_PREPARE],
[AS_UTIL_DEFUN_PROG([BASENAME], [basename])dnl
AS_UTIL_DEFUN_PROG([CAT], [cat])dnl
AS_UTIL_DEFUN_PROG([CHMOD], [chmod])dnl
AS_UTIL_DEFUN_PROG([CHOWN], [chown])dnl
AS_UTIL_DEFUN_PROG([CKSUM], [cksum])dnl
AS_UTIL_DEFUN_PROG([COMM], [comm])dnl
AS_UTIL_DEFUN_PROG([CP], [cp])dnl
AS_UTIL_DEFUN_PROG([CUT], [cut])dnl
AS_UTIL_DEFUN_PROG([DATE], [date])dnl
AS_UTIL_DEFUN_PROG([DIRNAME], [dirname])dnl
AS_UTIL_DEFUN_PROG([ECHO], [echo])dnl
AS_UTIL_DEFUN_PROG([ENV], [env])dnl
AS_UTIL_DEFUN_PROG([EXPR], [expr])dnl
AS_UTIL_DEFUN_PROG([FALSE], [false])dnl
AS_UTIL_DEFUN_PROG([GROUPS], [groups])dnl
AS_UTIL_DEFUN_PROG([HEAD], [head])dnl
AS_UTIL_DEFUN_PROG([INSTALL], [install])dnl
AS_UTIL_DEFUN_PROG([KILL], [kill])dnl
AS_UTIL_DEFUN_PROG([LN], [ln])dnl
AS_UTIL_DEFUN_PROG([LS], [ls])dnl
AS_UTIL_DEFUN_PROG([MD5SUM], [md5sum])dnl
AS_UTIL_DEFUN_PROG([MKDIR], [mkdir])dnl
AS_UTIL_DEFUN_PROG([MKTEMP], [mktemp])dnl
AS_UTIL_DEFUN_PROG([MV], [mv])dnl
AS_UTIL_DEFUN_PROG([NICE], [nice])dnl
AS_UTIL_DEFUN_PROG([PRINTENV], [printenv])dnl
AS_UTIL_DEFUN_PROG([PRINTF], [printf])dnl
AS_UTIL_DEFUN_PROG([PWD], [pwd])dnl
AS_UTIL_DEFUN_PROG([READLINK], [readlink])dnl
AS_UTIL_DEFUN_PROG([REALPATH], [realpath])dnl
AS_UTIL_DEFUN_PROG([RM], [rm])dnl
AS_UTIL_DEFUN_PROG([RMDIR], [rmdir])dnl
AS_UTIL_DEFUN_PROG([SEQ], [seq])dnl
AS_UTIL_DEFUN_PROG([SHA256SUM], [sha256sum])dnl
AS_UTIL_DEFUN_PROG([SLEEP], [sleep])dnl
AS_UTIL_DEFUN_PROG([SORT], [sort])dnl
AS_UTIL_DEFUN_PROG([STAT], [stat])dnl
AS_UTIL_DEFUN_PROG([SUM], [sum])dnl
AS_UTIL_DEFUN_PROG([SYNC], [sync])dnl
AS_UTIL_DEFUN_PROG([TAIL], [tail])dnl
AS_UTIL_DEFUN_PROG([TEE], [tee])dnl
AS_UTIL_DEFUN_PROG([TEST], [test])dnl
AS_UTIL_DEFUN_PROG([TOUCH], [touch])dnl
AS_UTIL_DEFUN_PROG([TR], [tr])dnl
AS_UTIL_DEFUN_PROG([TRUE], [true])dnl
AS_UTIL_DEFUN_PROG([UNAME], [uname])dnl
AS_UTIL_DEFUN_PROG([UNIQ], [uniq])dnl
AS_UTIL_DEFUN_PROG([USERS], [users])dnl
AS_UTIL_DEFUN_PROG([WC], [wc])dnl
AS_UTIL_DEFUN_PROG([WHOAMI], [whoami])dnl
AS_UTIL_DEFUN_PROG([YES], [yes])dnl
])# _AS_UTILS_COREUTILS_PREPARE

m4_defun([_AS_UTILS_UNIX_PROGS_PREPARE],
[AS_UTIL_DEFUN_PROG([SUDO], [sudo])dnl
AS_UTIL_DEFUN_PROG([SED], [sed])dnl
AS_UTIL_DEFUN_PROG([ED], [ed])dnl
AS_UTIL_DEFUN_PROG([PATCH], [patch])dnl
AS_UTIL_DEFUN_PROG([GREP], [grep])dnl
AS_UTIL_DEFUN_PROG([AWK], [awk])dnl
AS_UTIL_DEFUN_PROG([FIND], [find])dnl
AS_UTIL_DEFUN_PROG([XARGS], [xargs])dnl
])# _AS_UTILS_UNIX_PROGS_PREPARE


# ---------------------------------------------------------------------------- #

m4_defun([_AS_UTILS_INSTALL_DATA],
[AS_USE_PROG([INSTALL])dnl
AS_REQUIRE_SHELL_FN([as_fn_install_data],
  [AS_FUNCTION_DESCRIBE([as_fn_install_data], [FILE... TARGET],
    [Install data FILE(s) to TARGET location using INSTALL -m 0644. ]
    [Missing directories will be created automatically.])],
  [local _as_util_install_data_target
  eval _as_util_install_data_target="\$$@%:@"
  AS_UTIL_ASSERT_PROG([INSTALL])
  AS_MKDIR_P(["$( AS_DIRNAME(["$_as_util_install_data_target"]); )"])
  AS_CASE(["$@%:@"], [2], [${INSTALL:?} -m 0644 "$[]1" "$[]2"],
                     [${INSTALL:?} -m 0644 "$[]@"])])dnl
])# _AS_UTILS_INSTALL_DATA

# AS_INSTALL_DATA(FILE..., TARGET)
# --------------------------------
# Install data FILE(s) to TARGET using $INSTALL -m 0644.
# Create missing directories.
m4_define([AS_INSTALL_DATA],
  [AS_REQUIRE([_AS_UTILS_INSTALL_DATA])as_fn_install_data $@[]dnl
])# AS_INSTALL_DATA


# ---------------------------------------------------------------------------- #

m4_define([AS_UTIL_PUSH_SUDO_PROG],
[AS_UTIL_REQUIRE_PROG([SUDO], [sudo])[]dnl
m4_divert_once([DEFAULTS-STASH],
  [: "${_as_util_sp_$1_ORIG:=$[]$1}"; export _as_util_sp_$1_ORIG])dnl
$1="$SUDO -E -u m4_default_nblank([$2], [root]) -- $[]_as_util_sp_$1_ORIG"
])# AS_UTIL_PUSH_SUDO_PROG

m4_define([AS_UTIL_POP_SUDO_PROG],
[AS_UTIL_REQUIRE_PROG([SUDO], [sudo])[]dnl
$1="$_as_util_sp_$1_ORIG"
])# AS_UTIL_PUSH_SUDO_PROG

m4_define([AS_UTIL_PUSH_SUDO_PROGS],
[m4_map_args_sep([AS_UTIL_PUSH_SUDO_PROG(], [, $1)], [], m4_shift($@))dnl
])# AS_UTIL_PUSH_SUDO_PROGS

m4_define([AS_UTIL_POP_SUDO_PROGS],
[m4_map_args([AS_UTIL_POP_SUDO_PROG], $@)[]dnl
])# AS_UTIL_POP_SUDO_PROGS


# ---------------------------------------------------------------------------- #

# AS_UTIL_INIT
# ------------
m4_define([AS_UTIL_INIT],
[m4_divert_push([KILL])dnl
m4_divert_text([DEFAULTS], [@%:@ Defaults for shell variables])dnl
m4_divert_text([DEFAULTS-STASH], [
@%:@ Backup original values for shell variables])dnl
dnl Run required dependency checks
m4_divert_text([REQUIRE-DEPS], [
@%:@ Assert that required dependencies are available])dnl
m4_divert_text([PRE-BODY-SPACING], [

])dnl
m4_divert_pop([KILL])dnl
AS_REQUIRE([_AS_UTILS_COREUTILS_PREPARE])dnl
AS_REQUIRE([_AS_UTILS_UNIX_PROGS_PREPARE])dnl
m4_pattern_forbid([_AS_UTIL_])dnl
m4_provide([AS_UTIL_INIT])dnl
])# AS_UTIL_INIT


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
# vim: set filetype=config :
