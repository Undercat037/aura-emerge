# aura-emerge
[en](README.MD) | ua

> Обгортка у стилі Gentoo `emerge` для Arch Linux, побудована на основі [Aura](https://github.com/fosskers/aura).

Керує пакунками через файл `world.set` — встановив один раз, відстежується назавжди.

## Встановлення

```bash
git clone https://github.com/Undercat037/aura-emerge
cd aura-emerge
cargo build --release
sudo cp target/release/aura-emerge /usr/local/bin/emerge
sudo ln -sf /usr/local/bin/emerge /usr/local/bin/portageq
sudo mkdir -p /etc/emerge
sudo touch /etc/emerge/world.set
```
або через AUR:
```bash
paru -S aura-emerge 
```
[Релізний бінарник](target/release/)

**Вимоги:** `aura` має бути встановлена та доступна в `/usr/bin/aura`.  
**Опційно:** `asp` для підтримки `--abs` (`emerge asp`).

## Gentoo → Arch: аналог make.conf

В Arch Linux немає USE-прапорів, але налаштувати компіляцію можна схоже до [Gentoo](https://wiki.gentoo.org/wiki/Main_Page) `/etc/portage/make.conf` — через `/etc/makepkg.conf`:

```bash
sudo nvim /etc/makepkg.conf
```

Основні опції, що відповідають концепціям Gentoo:

| Gentoo (`make.conf`)         | Arch (`makepkg.conf`)         | Призначення                        |
|------------------------------|-------------------------------|-------------------------------------|
| `CFLAGS="-O2 -march=native"` | `CFLAGS="-O2 -march=native"`  | Прапори оптимізації компілятора     |
| `CXXFLAGS="-O2 -march=..."`  | `CXXFLAGS="-O2 -march=..."`   | Прапори компілятора C++             |
| `RUSTFLAGS="-C opt-level=3"` | `RUSTFLAGS="-C opt-level=3"`  | Прапори компілятора Rust            |
| `MAKEOPTS="-j$(nproc)"`      | `MAKEFLAGS="-j$(nproc)"`      | Паралельна збірка                   |
| `USE="lto"`                  | `OPTIONS+=(lto)`              | Link-time optimisation              |
| `FEATURES="ccache"`          | `BUILDENV+=(ccache)`          | Кеш збірки                          |
| `PORTDIR` / `PKGDIR`         | `PKGDEST`, `SRCDEST`          | Директорії артефактів збірки        |

> **Примітка:** USE-прапори в Arch відсутні — пакунки компілюються з фіксованим набором опцій від мейнтейнера. Щоб додати або прибрати функціональність, можна підправити PKGBUILD з ABS (`emerge --abs <pkg>`) перед збіркою.

Довідка по `CFLAGS`/`CXXFLAGS` — [прапори оптимізації GCC](https://gcc.gnu.org/onlinedocs/gcc/Optimize-Options.html), по `RUSTFLAGS` — [параметри кодогенерації rustc](https://doc.rust-lang.org/rustc/codegen-options/index.html).

[**→ Приклад makepkg.conf**](makepkg.conf)

## Використання

```bash
emerge nano                  # встановити пакунок (офіційні репо, автоматичний пошук в AUR)
emerge nano --aur            # встановити виключно з AUR
emerge nano --abs            # зібрати з ABS (Arch Build System) через asp
emerge nano --abs --skippgp  # зібрати з ABS, пропустити перевірку PGP підпису
emerge -c                    # видалити сиріт (depclean)
emerge -C nano               # видалити конкретний пакунок (unmerge)
emerge -u @world             # оновити всю систему (офіційні + AUR)
emerge --sync                # синхронізувати бази даних
emerge -s nano               # пошук в офіційних репо + AUR
emerge --searchdesc nano     # пошук за описом пакунка
emerge -sv nano              # детальна інформація (aura -Si / -Ai)
emerge -sv nano --aur        # детальна інформація виключно з AUR
emerge -1 nano               # встановити як залежність (не додається у world.set)
emerge -n nano               # не перевстановлювати, якщо вже встановлено (--noreplace)
emerge -p nano               # показати, що буде зроблено (dry run)
emerge -a nano               # запитати перед встановленням
emerge --select nano         # додати до world.set без встановлення
emerge --deselect nano       # прибрати з world.set без видалення
emerge --prune               # видалити пакунки, яких немає у world.set
emerge --regen               # оновити кеш метаданих пакунків
emerge --regen-world         # оновити префікси репозиторіїв у world.set
emerge --resume              # продовжити перерваний процес
emerge --resume --skipfirst  # продовжити, пропустивши перший пакунок
```

## world.set

Файл відстеження знаходиться за шляхом `/etc/emerge/world.set`. Кожен встановлений пакунок додається туди автоматично, а при видаленні — прибирається. Ви можете редагувати його власноруч (один пакунок на рядок).

Пакунки зберігаються з префіксом репозиторію для повної прозорості:

```
cachyos-extra-v3/rust-analyzer
chaotic-aur/visual-studio-code-bin
core/nano
aur/ayugram-desktop-bin
abs/nano-custom
extra/firefox
```

Префікси: назва офіційного репозиторію, `aur/` для AUR, `abs/` для зібраних з ABS локально, `Err/` якщо джерело не вдалося визначити.

## Прапори

| Прапор | Опис |
|--------|------|
| `-s` | Пошук пакета |
| `--searchdesc` | Пошук за описом |
| `-sv` | Детальна інформація про пакет |
| `-u` | Оновити систему (@world) |
| `-C` | Видалити конкретні пакети (unmerge) |
| `-c` | Видалити сиріт (depclean) |
| `-p` | Симуляція (dry run) |
| `-a` | Запит підтвердження |
| `-1` | Встановити як залежність (oneshot) |
| `-v` | Детальний вивід (verbose) |
| `-n` | Не перевстановлювати (noreplace) |
| `--aur` | Примусово використати AUR |
| `--abs` | Зібрати з ABS через asp |
| `--skippgp` | Пропустити перевірку PGP при збірці з ABS |
| `--sync` | Синхронізувати бази даних |
| `--select` | Додати до world.set (без встановлення) |
| `--deselect` | Прибрати з world.set (без видалення) |
| `--prune` | Видалити пакунки поза world.set |
| `--regen` | Оновити кеш метаданих |
| `--regen-world` | Оновити префікси репо у world.set |
| `--resume` | Продовжити перерваний процес |
| `--skipfirst` | Пропустити перший пакунок при resume |
| `--with-bdeps` | Підтягнути залежності збірки (сумісність) |
| `--verbose-conflicts` | Детальні конфлікти слотів (сумісність) |
| `-h` / `--help` | Показати довідку |
| `-V` / `--version` | Показати версію |

## Автор

**Undercat037**