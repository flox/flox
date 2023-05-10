zprofile=${FLOX_ORIG_ZDOTDIR:-$HOME}/.zprofile
if [ -f ${zprofile} ]
then
    ZDOTDIR=${FLOX_ORIG_ZDOTDIR} FLOX_ORIG_ZDOTDIR= source ${zprofile}
fi
