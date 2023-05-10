{}: src: let
  prefix =
    if src ? revCount
    then "r"
    else "";
  revision = src.revCount or src.shortRev or "dirty";
in "${prefix}${toString revision}"
