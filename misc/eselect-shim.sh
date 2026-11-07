#!/bin/bash
# aura-emerge: manage the optional `eselect` shim.
#
#   eselect-shim install   create /usr/local/bin/eselect -> emerge (+ completions)
#                          unless a real eselect (ae-eselect, ...) is installed
#   eselect-shim remove    delete only what this script created
#
# The shim lives in /usr/local so pacman never sees a file conflict when
# ae-eselect later installs the real /usr/bin/eselect. /usr/local/bin
# precedes /usr/bin in PATH, hence `install` backs off (and cleans up)
# whenever /usr/bin/eselect exists.
#
# AE_SHIM_SRC overrides where the completion files are read from.

SRC=${AE_SHIM_SRC:-/usr/share/aura-emerge/eselect-shim}
MARKER='aura-emerge eselect shim'
LINK=/usr/local/bin/eselect

# "source-name:destination"
FILES=(
  "eselect.bash-completion:/usr/local/share/bash-completion/completions/eselect"
  "_eselect:/usr/local/share/zsh/site-functions/_eselect"
  "eselect.fish:/usr/local/share/fish/vendor_completions.d/eselect.fish"
)

is_emerge_link() {
  local t
  [[ -L $1 ]] || return 1
  t=$(readlink -- "$1") || return 1
  [[ $t == emerge || $t == /usr/bin/emerge ]]
}

shim_remove() {
  local e f
  if is_emerge_link "$LINK"; then rm -f -- "$LINK"; fi
  for e in "${FILES[@]}"; do
    f=${e#*:}
    if [[ -f $f ]] && grep -qF -- "$MARKER" "$f"; then rm -f -- "$f"; fi
  done
}

shim_install() {
  local e src dst

  # Legacy (<= 3.0.35): untracked link in /usr/bin. It would make pacman
  # refuse to install ae-eselect, so move it to /usr/local.
  if is_emerge_link /usr/bin/eselect; then rm -f -- /usr/bin/eselect; fi

  # A real eselect wins; make sure our shim doesn't shadow it.
  if [[ -e /usr/bin/eselect || -L /usr/bin/eselect ]]; then
    shim_remove
    return 0
  fi

  # Someone else's file in /usr/local/bin: hands off.
  if [[ -e $LINK || -L $LINK ]] && ! is_emerge_link "$LINK"; then
    return 0
  fi

  install -d -m755 /usr/local/bin
  ln -sfn /usr/bin/emerge "$LINK"

  for e in "${FILES[@]}"; do
    src=$SRC/${e%%:*}
    dst=${e#*:}
    [[ -f $src ]] || continue
    # keep a user's own completion file
    if [[ -e $dst ]] && ! grep -qF -- "$MARKER" "$dst"; then continue; fi
    install -Dm644 "$src" "$dst"
  done
}

case ${1:-} in
  install) shim_install ;;
  remove) shim_remove ;;
  *)
    echo "usage: ${0##*/} install|remove" >&2
    exit 2
    ;;
esac
exit 0
