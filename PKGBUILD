# Maintainer: Undercat037 <deltacatdeveloper@gmail.com>
pkgname=aura-emerge-git
pkgver=3.0.17_indev_beta.r201.gb8158b7
pkgrel=1
pkgdesc="A standalone Gentoo-style emerge for Arch Linux - installs from official repos, the AUR, and ABS, scans PKGBUILDs for supply-chain red flags before building, and runs untrusted build steps inside a bwrap sandbox. (git version)"
arch=('x86_64')
url="https://undercat037.github.io/aura-emerge/"
# Github repo: https://github.com/Undercat037/aura-emerge
# Gitlab repo: https://gitlab.com/Undercat037/aura-emerge
license=('GPL-3.0-only')
depends=('git' 'sudo' 'bubblewrap' 'pacman')
optdepends=('devtools: for --abs support (pkgctl repo clone)'
            'gnupg: for PGP verification when building from ABS')
makedepends=('rust' 'cargo')
conflicts=('portage' 'portage-git' 'eselect-git' 'aura-emerge')
provides=('emerge' 'portageq' 'eselect')
install=aura-emerge.install
backup=('etc/portage/world')
source=("$pkgname::git+https://github.com/Undercat037/aura-emerge.git")
sha256sums=('SKIP')

pkgver() {
  cd "$pkgname"
  local ver
  ver=$(grep '^version' Cargo.toml | head -1 | sed 's/version = "\(.*\)"/\1/; s/-/_/g')
  printf '%s.r%s.g%s' "$ver" "$(git rev-list --count HEAD)" "$(git rev-parse --short HEAD)"
}

build() {
  cd "$pkgname"
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
  cd "$pkgname"

  install -Dm755 target/release/aura-emerge "$pkgdir/usr/bin/emerge"
  install -Dm644 LICENSE "$pkgdir/usr/share/licenses/$pkgname/LICENSE"

  # docs + example configs
  install -Dm644 README.MD "$pkgdir/usr/share/doc/$pkgname/README.md"
  install -Dm644 RU-README.MD "$pkgdir/usr/share/doc/$pkgname/RU-README.md"
  install -Dm644 UA-README.MD "$pkgdir/usr/share/doc/$pkgname/UA-README.md"
  install -Dm644 misc/make.conf "$pkgdir/usr/share/doc/$pkgname/examples/make.conf"
  install -Dm644 misc/makepkg.conf "$pkgdir/usr/share/doc/$pkgname/examples/makepkg.conf"
  install -Dm644 misc/package.mask "$pkgdir/usr/share/doc/$pkgname/examples/package.mask"

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
