# Source /etc/zlogout and "${FLOX_ORIG_ZDOTDIR:-$HOME}/.zlogout" if they exist.
#
# See README.md for more information on the initialization process.

zlogout="${FLOX_ORIG_ZDOTDIR:-$HOME}/.zlogout"
if [ -f "${zlogout}" ]
then
    ZDOTDIR="${FLOX_ORIG_ZDOTDIR}" FLOX_ORIG_ZDOTDIR= source "${zlogout}"
fi

if [ -f /etc/zlogout ]
then
    ZDOTDIR="${FLOX_ORIG_ZDOTDIR}" FLOX_ORIG_ZDOTDIR= source /etc/zlogout
fi
