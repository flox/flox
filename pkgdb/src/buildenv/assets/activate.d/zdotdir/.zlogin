# Source /etc/zlogin and "${FLOX_ORIG_ZDOTDIR:-$HOME}/.zlogin" if they exist.
#
# See README.md for more information on the initialization process.

if [ -f /etc/zlogin ]
then
    ZDOTDIR="${FLOX_ORIG_ZDOTDIR}" FLOX_ORIG_ZDOTDIR= source /etc/zlogin
fi

zlogin="${FLOX_ORIG_ZDOTDIR:-$HOME}/.zlogin"
if [ -f "${zlogin}" ]
then
    ZDOTDIR="${FLOX_ORIG_ZDOTDIR}" FLOX_ORIG_ZDOTDIR= source "${zlogin}"
fi
