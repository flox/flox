if [ -f /etc/zlogin ]
then
    ZDOTDIR="${FLOX_ORIG_ZDOTDIR}" FLOX_ORIG_ZDOTDIR= source /etc/zlogin
fi

zlogin="${FLOX_ORIG_ZDOTDIR:-$HOME}/.zlogin"
if [ -f "${zlogin}" ]
then
    ZDOTDIR="${FLOX_ORIG_ZDOTDIR}" FLOX_ORIG_ZDOTDIR= source "${zlogin}"
fi
