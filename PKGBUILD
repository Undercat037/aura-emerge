# Maintainer: Undercat037 <deltacatdeveloper@gmail.com>
pkgname=aura-emerge
pkgver=0.00.0
pkgrel=1
pkgdesc="Portage-like wrapper for Arch Linux using Aura"
arch=('x86_64')
url="https://github.com/Undercat037/aura-emerge"
license=('GPL-3.0')
depends=('aura')
optdepends=('asp: for --abs support' 'gnupg: for PGP verification')
makedepends=('rust' 'cargo')
conflicts=('portage')
install=aura-emerge.install
backup=('etc/emerge/world.set')
source=("$pkgname-$pkgver.tar.gz::https://github.com/Undercat037/aura-emerge/archive/refs/heads/main.tar.gz")
sha256sums=('SKIP')

pkgver() {
  cd "aura-emerge-main"
  grep '^version' Cargo.toml | head -1 | sed 's/version = "\(.*\)"/\1/'
}

build() {
  cd "aura-emerge-main"
  cargo build --release
}

package() {
  cd "aura-emerge-main"
  local bin="target/release/aura-emerge"

  install -Dm755 target/release/aura-emerge "$pkgdir/usr/bin/emerge"
  install -Dm644 LICENSE "$pkgdir/usr/share/licenses/$pkgname/LICENSE"
  install -Dm644 README.MD "$pkgdir/usr/share/doc/$pkgname/README.md"
  install -Dm644 UA-README.MD "$pkgdir/usr/share/doc/$pkgname/UA-README.md"
  install -Dm644 <("$bin" --gen-manpage) "$pkgdir/usr/share/man/man1/emerge.1"

  install -dm755 "$pkgdir/etc/emerge"
  install -dm755 "$pkgdir/etc/emerge/sets.d"
  install -Dm644 /dev/null "$pkgdir/etc/emerge/world.set"

  install -Dm644 <("$bin" --gen-completions bash) \
    "$pkgdir/usr/share/bash-completion/completions/emerge"
  install -Dm644 <("$bin" --gen-completions zsh) \
    "$pkgdir/usr/share/zsh/site-functions/_emerge"
  install -Dm644 <("$bin" --gen-completions fish) \
    "$pkgdir/usr/share/fish/vendor_completions.d/emerge.fish"
}
