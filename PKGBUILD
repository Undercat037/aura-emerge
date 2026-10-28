# Maintainer: Undercat037 <deltacatdeveloper@gmail.com>
pkgname=aura-emerge-git
pkgver=0.0.0
pkgrel=1
pkgdesc="A standalone Gentoo-style emerge for Arch Linux - installs from official repos, the AUR, and ABS, scans PKGBUILDs for supply-chain red flags before building, and runs untrusted build steps inside a bwrap sandbox. (git version)"
arch=('x86_64')
url="https://undercat037.github.io/aura-emerge/"
# Github repo: https://github.com/Undercat037/aura-emerge
# Gitlab repo: https://gitlab.com/Undercat037/aura-emerge
license=('GPL-3.0-only')
depends=('git' 'sudo' 'bubblewrap' 'pacman' 'gnupg')
optdepends=('devtools: for --abs support (pkgctl repo clone)')
makedepends=('rust' 'cargo')
conflicts=('portage' 'portage-git' 'eselect-git' 'aura-emerge')
provides=('emerge' 'portageq' 'eselect')
install=aura-emerge.install
backup=('etc/portage/world' 'etc/pacman.d/aura-emerge-repos.conf')
source=()
sha256sums=()

pkgver() {
  cd "${startdir:-${PWD}}" || return 1
  local ver rev short pkgid

  # path+file:///…/crate#3.0.28-beta
  # path+file:///…/crate#name@3.0.28-beta
  if command -v cargo >/dev/null 2>&1; then
    pkgid=$(cargo pkgid 2>/dev/null) || pkgid=
    if [[ -n $pkgid && $pkgid == *#* ]]; then
      if [[ $pkgid == *#*@* ]]; then
        ver=${pkgid##*@}
      else
        ver=${pkgid##*#}
      fi
      ver=${ver%%[[:space:]]*}
    fi
  fi

  if [[ -z $ver && -f Cargo.toml ]]; then
    ver=$(awk '
      /^\[package\]/ { in_pkg = 1; next }
      /^\[/ { in_pkg = 0 }
      in_pkg && $0 ~ /^[[:space:]]*version[[:space:]]*=/ {
        line = $0
        sub(/^[^=]*=[[:space:]]*/, "", line)
        gsub(/["\047]/, "", line)
        sub(/[[:space:]].*/, "", line)
        sub(/,.*/, "", line)
        print line
        exit
      }
    ' Cargo.toml)
  fi

  ver=${ver//-/_}
  [[ -n $ver ]] || return 1

  if rev=$(git rev-list --count HEAD 2>/dev/null) && short=$(git rev-parse --short HEAD 2>/dev/null); then
    printf '%s.r%s.g%s' "$ver" "$rev" "$short"
  else
    printf '%s' "$ver"
  fi
}

build() {
  cd "$startdir" || return 1
  cargo build --release --locked

  local bin="$PWD/target/release/aura-emerge"
  "$bin" --gen-manpage >man.1
  "$bin" --gen-completions bash >comp.bash
  "$bin" --gen-completions zsh >comp.zsh
  "$bin" --gen-completions fish >comp.fish
  for f in man.1 comp.bash comp.zsh comp.fish; do
    [[ -s $f ]] || {
      error "empty $f - refusing to package it"
      return 1
    }
  done
}

package() {
  cd "$startdir" || return 1

  install -Dm755 target/release/aura-emerge "$pkgdir/usr/bin/emerge"
  install -Dm644 LICENSE "$pkgdir/usr/share/licenses/$pkgname/LICENSE"

  # docs + example configs
  install -Dm644 README.MD "$pkgdir/usr/share/doc/$pkgname/README.md"
  install -Dm644 RU-README.MD "$pkgdir/usr/share/doc/$pkgname/RU-README.md"
  install -Dm644 UA-README.MD "$pkgdir/usr/share/doc/$pkgname/UA-README.md"
  install -Dm644 misc/make.conf "$pkgdir/usr/share/doc/$pkgname/examples/make.conf"
  install -Dm644 misc/makepkg.conf "$pkgdir/usr/share/doc/$pkgname/examples/makepkg.conf"
  install -Dm644 misc/package.mask "$pkgdir/usr/share/doc/$pkgname/examples/package.mask"

  # local repos (FEATURES="localrepo")
  install -Dm644 misc/ae-repos.conf "$pkgdir/etc/portage/ae-repos.conf"

  # install manpage
  install -Dm644 man.1 "$pkgdir/usr/share/man/man1/emerge.1"

  # /etc/portage: world is tracked, sets/ and hooks/ are user dirs
  install -dm755 "$pkgdir/etc/portage/sets"
  install -dm755 "$pkgdir/etc/portage/hooks"
  install -Dm644 /dev/null "$pkgdir/etc/portage/world"

  # shell completions
  install -Dm644 comp.bash "$pkgdir/usr/share/bash-completion/completions/emerge"
  install -Dm644 comp.zsh "$pkgdir/usr/share/zsh/site-functions/_emerge"
  install -Dm644 comp.fish "$pkgdir/usr/share/fish/vendor_completions.d/emerge.fish"

  # eselect shim completions (profile module only)
  install -Dm644 misc/eselect.bash-completion "$pkgdir/usr/share/bash-completion/completions/eselect"
  install -Dm644 misc/_eselect "$pkgdir/usr/share/zsh/site-functions/_eselect"
  install -Dm644 misc/eselect.fish "$pkgdir/usr/share/fish/vendor_completions.d/eselect.fish"
}
