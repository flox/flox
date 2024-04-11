zlogout="${FLOX_ORIG_ZDOTDIR:-$HOME}/.zlogout"
if [ -f "${zlogout}" ]
then
    ZDOTDIR="${FLOX_ORIG_ZDOTDIR}" FLOX_ORIG_ZDOTDIR= source "${zlogout}"
fi

if [ -f /etc/zlogout ]
then
    ZDOTDIR="${FLOX_ORIG_ZDOTDIR}" FLOX_ORIG_ZDOTDIR= source /etc/zlogout
fi
