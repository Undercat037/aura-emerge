# Maintainer: Undercat037 <deltacatdeveloper@gmail.com>
pkgname=aura-emerge-git
pkgver=0.0.0
pkgrel=1
pkgdesc="A standalone Gentoo-style emerge for Arch Linux - installs from official repos, the AUR, and ABS, scans PKGBUILDs for supply-chain red flags before building, and runs untrusted build steps inside a bwrap sandbox."
arch=('x86_64')
url="https://undercat037.github.io/aura-emerge/"
# Github repo: https://github.com/Undercat037/aura-emerge
# Gitlab repo: https://gitlab.com/Undercat037/aura-emerge
license=('GPL-3.0-only')
depends=('git' 'sudo' 'bubblewrap')
optdepends=('devtools: for --abs support (pkgctl repo clone)'
  'gnupg: for PGP verification when building from ABS')
makedepends=('rust' 'cargo')
conflicts=('portage' 'portage-git' 'aura-emerge')
provides=('portageq')
install=aura-emerge.install
backup=('etc/portage/world')
source=("$pkgname-$pkgver.tar.gz::https://github.com/Undercat037/aura-emerge/archive/refs/heads/main.tar.gz")
sha256sums=('SKIP')

pkgver() {
  cd "aura-emerge-main"
  grep '^version' Cargo.toml | head -1 | sed 's/version = "\(.*\)"/\1/; s/-/_/g'
}

build() {
  cd "aura-emerge-main"
  cargo build --release

  local bin="$PWD/target/release/aura-emerge"
  "$bin" --gen-manpage >man.1
  "$bin" --gen-completions bash >comp.bash
  "$bin" --gen-completions zsh >comp.zsh
  "$bin" --gen-completions fish >comp.fish
  for f in man.1 comp.bash comp.zsh comp.fish; do
    [[ -s $f ]] || {
      echo "empty $f"
      return 1
    }
  done
}

package() {
  cd "aura-emerge-main"
  install -Dm755 target/release/aura-emerge "$pkgdir/usr/bin/emerge"
  install -Dm644 LICENSE "$pkgdir/usr/share/licenses/$pkgname/LICENSE"
  install -Dm644 README.MD "$pkgdir/usr/share/doc/$pkgname/README.md"
  install -Dm644 UA-README.MD "$pkgdir/usr/share/doc/$pkgname/UA-README.md"
  install -Dm644 man.1 "$pkgdir/usr/share/man/man1/emerge.1"
  install -dm755 "$pkgdir/etc/portage/sets"
  install -Dm644 /dev/null "$pkgdir/etc/portage/world"
  install -Dm644 comp.bash "$pkgdir/usr/share/bash-completion/completions/emerge"
  install -Dm644 comp.zsh "$pkgdir/usr/share/zsh/site-functions/_emerge"
  install -Dm644 comp.fish "$pkgdir/usr/share/fish/vendor_completions.d/emerge.fish"
}
