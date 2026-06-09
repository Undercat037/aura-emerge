# Maintainer: Undercat037 <deltacatdeveloper@gmail.com>
pkgname=aura-emerge
pkgver=1.22.0
pkgrel=1
pkgdesc="Portage-like wrapper for Arch Linux using Aura (git)"
arch=('x86_64')
url="https://github.com/Undercat037/aura-emerge"
license=('GPL-3.0')
depends=('aura')
optdepends=('asp: for --abs support (build from ABS source)'
            'gnupg: for PGP verification when building from ABS')
makedepends=('rust' 'cargo' 'git')
provides=('aura-emerge')
conflicts=('aura-emerge' 'portageq-shim')
backup=('etc/emerge/world.set')
source=("$pkgname::git+https://github.com/Undercat037/aura-emerge.git")
sha256sums=('SKIP')

pkgver() {
  cd "$pkgname"
  # Pull version directly from Cargo.toml — no manual bumps needed
  grep '^version' Cargo.toml | head -1 | sed 's/version = "\(.*\)"/\1/'
}

prepare() {
  for f in /usr/local/bin/emerge /usr/local/bin/portageq; do
    if [[ -e "$f" ]] && ! pacman -Qo "$f" &>/dev/null; then
      rm -f "$f"
    fi
  done
}

build() {
  cd "$pkgname"
  cargo build --release
}

package() {
  cd "$pkgname"
  install -Dm755 target/release/aura-emerge "$pkgdir/usr/local/bin/emerge"
  ln -sf /usr/local/bin/emerge "$pkgdir/usr/local/bin/portageq"
  install -Dm644 LICENSE "$pkgdir/usr/share/licenses/${pkgname%-git}/LICENSE"
  install -Dm644 README.MD "$pkgdir/usr/share/doc/${pkgname%-git}/README.md"
  install -dm755 "$pkgdir/etc/emerge"
  install -Dm644 /dev/null "$pkgdir/etc/emerge/world.set"
}