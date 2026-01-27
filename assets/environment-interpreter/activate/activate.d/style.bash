allows_color() {
  if [[ -t 1 ]] && [[ "${NO_COLOR:-0}" == "0" ]]; then
    return 0
  else
    return 1
  fi
}

wrap_color() {
  text="$1"
  color="$2"
  use_color="$3"
  if [[ "$use_color" == 1 ]]; then
    color_green="\x1b[32m"
    color_red="\x1b[31m"
    color_yellow="\x1b[33m"
    color_blue="\x1b[34m"
    color_reset="\x1b[39m"
    case "$color" in
      green)
        echo -e "$color_green$text$color_reset"
        ;;
      red)
        echo -e "$color_red$text$color_reset"
        ;;
      yellow)
        echo -e "$color_yellow$text$color_reset"
        ;;
      blue)
        echo -e "$color_blue$text$color_reset"
        ;;
      *)
        echo "$text"
        ;;
    esac
  else
    echo "$text"
  fi
}

wrap_green() {
  wrap_color "$1" green "$2"
}

wrap_red() {
  wrap_color "$1" red "$2"
}

wrap_yellow() {
  wrap_color "$1" yellow "$2"
}

wrap_blue() {
  wrap_color "$1" blue "$2"
}

green_check() {
  wrap_green ✔ "$1"
}

red_x() {
  wrap_red ✘ "$1"
}

yellow_bang() {
  wrap_yellow ! "$1"
}

blue_i() {
  wrap_blue ℹ "$1"
}

yellow_bolt() {
  wrap_yellow ⚡︎ "$1"
}

red_minus() {
  wrap_red ━ "$1"
}
