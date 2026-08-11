# aura-emerge
[en](README.MD) | ua

> Автономний ПМ у стилі Gentoo `emerge` для Arch Linux - встановлює з офіційних репозиторіїв, AUR та ABS, сканує PKGBUILD на ознаки атак на ланцюг постачання перед збіркою і виконує недовірені кроки збірки всередині пісочниці `bwrap`.

Керує пакунками через файл `world.set` - встановив один раз, відстежується назавжди.

## Встановлення

### Makepkg
```bash
git clone https://github.com/Undercat037/aura-emerge
cd aura-emerge
makepkg -si
```

### AUR
```bash
paru -S aura-emerge
```

### Ручна збірка
```bash
git clone https://github.com/Undercat037/aura-emerge
cd aura-emerge
cargo build --release
sudo cp target/release/aura-emerge /usr/bin/emerge
sudo ln -sf /usr/bin/emerge /usr/bin/portageq
emerge --gen-completions bash   # → /usr/share/bash-completion/completions/emerge
emerge --gen-completions zsh    # → /usr/share/zsh/site-functions/_emerge
emerge --gen-completions fish   # → /usr/share/fish/vendor_completions.d/emerge.fish
emerge --gen-manpage | sudo tee /usr/share/man/man1/emerge.1 >/dev/null
```

**Залежності:** `git`  
**Опційно:**
- `devtools` - підтримка `--abs` (`pkgctl repo clone`)
- `gnupg` - автоматичний імпорт PGP-ключів (`--autopgp`)
- `bubblewrap` - пісочниця при збірці AUR/ABS (без неї - відкат до звичайного `makepkg` без ізоляції)

## Gentoo → Arch: аналог make.conf

В Arch Linux немає USE-прапорів, але налаштувати компіляцію можна схоже до [Gentoo](https://wiki.gentoo.org/wiki/Main_Page) `/etc/portage/make.conf` - через `/etc/makepkg.conf`:

```bash
sudo nvim /etc/makepkg.conf
```

| Gentoo (`make.conf`)         | Arch (`makepkg.conf`)         | Призначення                     |
|------------------------------|-------------------------------|---------------------------------|
| `CFLAGS="-O2 -march=native"` | `CFLAGS="-O2 -march=native"`  | Прапори оптимізації компілятора |
| `CXXFLAGS="-O2 -march=..."`  | `CXXFLAGS="-O2 -march=..."`   | Прапори компілятора C++         |
| `RUSTFLAGS="-C opt-level=3"` | `RUSTFLAGS="-C opt-level=3"`  | Прапори компілятора Rust        |
| `MAKEOPTS="-j$(nproc)"`      | `MAKEFLAGS="-j$(nproc)"`      | Паралельна збірка               |
| `USE="lto"`                  | `OPTIONS+=(lto)`              | Link-time optimisation          |
| `FEATURES="ccache"`          | `BUILDENV+=(ccache)`          | Кеш збірки                      |
| `PORTDIR` / `PKGDIR`         | `PKGDEST`, `SRCDEST`          | Директорії артефактів збірки    |

Довідка: [прапори оптимізації GCC](https://gcc.gnu.org/onlinedocs/gcc/Optimize-Options.html), [параметри кодогенерації rustc](https://doc.rust-lang.org/rustc/codegen-options/index.html).

[**→ Приклад makepkg.conf**](repo-files/makepkg.conf)

## Використання

```bash
emerge neovim                              # офіційні репо, при відсутності - AUR
emerge neovim-git --aur                    # лише AUR (без рекурсії AUR-залежностей)
emerge foo-git --aur --aur-deep            # також рекурсивно зібрати AUR-only залежності (--aur-full)
emerge neovim --only-repos                 # лише офіційні репо, без AUR
emerge neovim --abs                        # зібрати з ABS (pkgctl), у пісочниці bwrap
emerge neovim --abs --no-sandbox           # ABS без пісочниці (звичайний makepkg -si)
emerge neovim --abs --skippgp              # ABS, пропустити перевірку PGP
emerge neovim --abs --autopgp              # ABS, автоімпорт відсутніх PGP-ключів
emerge neovim --abs --edit                 # ABS, редагувати PKGBUILD ($EDITOR), оновлює .SRCINFO
emerge neovim-git --aur --edit             # AUR, редагувати PKGBUILD, оновлює .SRCINFO
emerge neovim-git --aur --edit --off-src-regen  # редагувати без оновлення .SRCINFO
emerge neovim-git --aur --unshare-net-build     # збірка без мережі на етапі build()/check()/package()
emerge -c                                  # видалити сиріт (depclean)
emerge -C neovim                           # видалити пакунок (unmerge)
emerge @world                              # provision: встановити відсутнє з world.set
emerge @world --err-inst                   # також спробувати записи Err/ звичайним шляхом
emerge -u @world                           # повне оновлення (репо + AUR); world.set не читається
emerge -u @world --refresh                 # оновлення з примусовим -Syy
emerge --sync                              # синхронізувати бази
emerge @game-kit                           # власний набір з /etc/emerge/sets.d/game-kit.set
emerge @game-kit neovim                    # набори + звичайні пакунки
emerge --list-sets                         # показати доступні @sets
emerge --info                              # інформація про систему (як Gentoo emerge --info)
emerge --news                              # новини Arch Linux, непрочитані позначені
emerge --news 3                            # прочитати новину 3
emerge --news all                          # позначити всі як прочитані
emerge -s neovim                           # пошук (репо + AUR)
emerge -s neovim --only-repos              # пошук лише в офіційних репо
emerge --searchdesc neovim                 # пошук за описом
emerge -sv neovim                          # детальна інформація
emerge -sv neovim-git --aur                # детальна інформація лише з AUR
emerge -1 htop                             # як залежність (не в world.set)
emerge -n neovim                           # не перевстановлювати (--noreplace)
emerge -p neovim                           # dry run
emerge -a neovim                           # запитати перед дією
emerge --select neovim                     # додати до world.set без встановлення
emerge --deselect neovim                   # прибрати з world.set без видалення
emerge --prune                             # видалити пакунки поза world.set
emerge --regen                             # оновити кеш метаданих
emerge --regen-world                       # оновити префікси репо у world.set
emerge --regen-sets @game-kit              # оновити префікси для одного набору
emerge --regen-sets @game-kit --regen-sort # те саме + сортування файлу
emerge --regen-world-from-explicit         # одноразова міграція з pacman -Qeq
emerge --resume                            # продовжити перерваний процес
emerge --resume --skipfirst                # продовжити, пропустивши перший пакунок
emerge --undo                              # скасувати останнє встановлення/видалення
emerge --clean-source-cache                # очистити кеш VCS-джерел (-git/-hg/-svn)
emerge @preserved-rebuild                  # перевірити відсутні залежності
emerge @preserved-rebuild -p               # лише показати список
emerge @preserved-rebuild -a               # запитувати перед кожним pacman
```

## Пісочниця

Збірки **AUR і ABS** виконують функції PKGBUILD `pkgver()` / `prepare()` / `build()` / `check()` / `package()` всередині простору імен [`bwrap`](https://github.com/containers/bubblewrap) (якщо встановлено `bubblewrap`):

- фейковий `$HOME` (немає SSH-ключів, GPG secret keyring, профілю браузера);
- запис лише в директорію збірки (+ налаштовані `PKGDEST` / `SRCDEST` / `SRCPKGDEST` / `BUILDDIR`);
- мережа спільна за замовчуванням (`--share-net`);
- решта просторів імен (user / IPC / PID / UTS / cgroup) знищена.

Це **другий, незалежний рівень** поверх [сканера PKGBUILD](#безпека-сканування-pkgbuild): сканер блокує *відомі* погані патерни до запуску, пісочниця обмежує наслідки того, що він пропустив.

Встановлення залежностей (`pacman -S`) і фінальний `pacman -U` лишаються **поза** пісочницею - там не виконується код з PKGBUILD.

| Прапор | Ефект |
|--------|--------|
| `--no-sandbox` | Вимкнути bwrap, звичайний `makepkg -si` |
| `--unshare-net-build` | Завантаження/`prepare()` - з мережею; `build()`/`check()`/`package()` - **без** мережі |

Без `bubblewrap` або з `--no-sandbox` збірка йде без ізоляції (з попередженням).

## Кеш джерел (SRCDEST)

Для `-git` / `-hg` / `-svn` пакунків aura-emerge за замовчуванням тримає постійний `SRCDEST` у `~/.cache/aura-emerge/sources` (або `$XDG_CACHE_HOME/aura-emerge/sources`). Makepkg робить інкрементальний `git fetch` замість повного клону на кожен запуск.

```bash
emerge --clean-source-cache   # видалити цей кеш
```

Якщо в `makepkg.conf` уже задано власний `SRCDEST` - використовується він.

## @preserved-rebuild

В Arch немає відстеження preserved-libs. `@preserved-rebuild` збирає залежності кожного встановленого пакунка (`pacman -Qi`), передає список у `pacman -T` і пропонує перевстановити незадоволені через `pacman -S --asdeps` (щоб вони не виглядали «явно встановленими» і не потрапили в world.set при `--regen-world-from-explicit`).

## world.set

Файл: `/etc/emerge/world.set`. Кожен явно встановлений пакунок додається автоматично, при видаленні - прибирається. Можна редагувати вручну (один пакунок на рядок).

```
cachyos-extra-v3/rust-analyzer
chaotic-aur/visual-studio-code-bin
core/nano
aur/ayugram-desktop-bin
abs/nano-custom
extra/firefox
```

Префікси: назва офіційного репо, `aur/`, `abs/`, `Err/` (джерело невизначене).

> **Про `Err/`:** використовується і для справді локальної збірки без походження, і для `abs/`/`aur/`, що згодом стали нерозпізнаваними. `--regen-world` / `--regen-sets` зберігають відомий префікс `abs/`/`aur/` замість зниження до `Err/` при першій невдалій спробі повторного розв’язання.

Встановлення кількох пакунків одночасно не зриває весь пакет, якщо один не знайдено - решта встановлюються, відсутні виводяться в кінці з ненульовим кодом виходу.

### `@world` має два значення

- **`emerge @world` (без `-u`)** - *provision*. Встановлює те, чого немає з `world.set`; уже встановлене не чіпається. Префікс визначає джерело. Зручно тримати `world.set` у dotfiles і на новій машині зробити `emerge @world`.
- **`emerge -u @world`** (або `emerge -u`) - *upgrade*. Оновлює все встановлене (репо + AUR). `world.set` не читається.
- **`emerge --sync`** - лише освіження баз (`-Sy` / `-Syy`).

## Власні набори (sets)

`/etc/emerge/sets.d/<name>.set` викликається як `@<name>`:

```
# /etc/emerge/sets.d/game-kit.set
steam
lutris
gamemode
aur/protonup-qt
```

Один пакунок на рядок, `#`-коментарі дозволені. Набори можна комбінувати:

```bash
emerge @game-kit @dev-tools neovim
```

`emerge --list-sets` і автодоповнення `emerge @<TAB>` показують доступні набори.

## --undo

Скасовує останнє успішне встановлення або видалення (один крок історії):

- **після install** - видаляє лише нові в цьому запуску пакунки;
- **після unmerge** - перевстановлює за записаним джерелом (`Err/`/`abs/` лише виводяться);
- **після `-u`** - не підтримується (немає знімка версій).

```bash
emerge --undo
```

## Новини

`emerge --news` завантажує [RSS-фід новин Arch Linux](https://archlinux.org/feeds/news/). Стан прочитаності - у `~/.cache/aura-emerge/news.state` (без root).

```bash
emerge --news        # список, непрочитані позначені [N]
emerge --news 3      # прочитати новину 3
emerge --news all    # позначити всі як прочитані
```

## Безпека: сканування PKGBUILD

Перед кожним встановленням з AUR завантажується «сирий» `PKGBUILD` (і `.install`-хук) з cgit і сканується у два шари:

1. **AST** (`tree-sitter-bash`) - `curl|sh`, `base64`/`xxd | sh`, `source <(curl …)`, `eval "$(curl …)"`, `sudo`/`pkexec`/`doas`, python inline-exec тощо. Тривіальні текстові обходи (конкатенація рядків, абсолютні шляхи, `env`) не проходять.
2. **Текстові евристики** - `chmod 777`, «сирі» IP, сторонні пакетні менеджери (`npm install …`), paste-сайти, `.onion`, відомі IOC (Atomic Arch, openconnect-sso, `compromised.txt` з 2018 тощо).

Знахідки: `Suspicious` або `ConfirmedIoc`. Спрацювання не блокує жорстко - показує файл/рядок, посилання на cgit, запит `Continue anyway? [y/N]`.

Збірки AUR і ABS додатково захищені [пісочницею `bwrap`](#пісочниця).

## Прапори

| Прапор | Опис |
|--------|------|
| `-s` | Пошук пакунка |
| `--searchdesc` | Пошук за описом |
| `-sv` | Детальна інформація |
| `-u` | Повне оновлення системи (world.set не читається) |
| `-C` | Видалити пакунки (unmerge) |
| `-c` | Видалити сиріт (depclean) |
| `-p` | Симуляція (dry run) |
| `-a` | Запит підтвердження |
| `-1` | Встановити як залежність (oneshot) |
| `-v` | Детальний вивід |
| `-n` | Не перевстановлювати (noreplace) |
| `--aur` | Примусово AUR |
| `--aur-deep` / `--aur-full` | Рекурсивно зібрати AUR-only залежності (неявно вмикає `--aur`) |
| `--only-repos` | Лише офіційні репо |
| `--abs` | Зібрати з ABS (`pkgctl`), у пісочниці bwrap |
| `--no-sandbox` | Вимкнути bwrap для AUR/ABS |
| `--unshare-net-build` | Без мережі на етапі build()/check()/package() |
| `--skippgp` | Пропустити перевірку PGP (ABS) |
| `--autopgp` | Автоімпорт відсутніх PGP-ключів (ABS) |
| `--edit` | Редагувати PKGBUILD перед збіркою; після збереження оновлює `.SRCINFO` |
| `--off-src-regen` | Разом з `--edit` - не оновлювати `.SRCINFO` |
| `--sync` | Синхронізувати бази |
| `--refresh` | Примусово `-Syy` (з `--sync` або `-u`) |
| `--info` | Інформація про систему |
| `--news [N\|all]` | Новини Arch Linux |
| `--check-news [N\|all]` | Синонім `--news` |
| `--select` | Додати до world.set без встановлення |
| `--deselect` | Прибрати з world.set без видалення |
| `--prune` | Видалити пакунки поза world.set |
| `--regen` | Оновити кеш метаданих |
| `--regen-world` | Оновити префікси репо у world.set |
| `--regen-sets <SET>` | Оновити префікси для одного набору |
| `--regen-sort` | Разом з `--regen-sets` - відсортувати файл |
| `--regen-world-from-explicit` | Міграція: наповнити world.set з `pacman -Qeq` |
| `--err-inst` | При `@world` також пробувати записи `Err/` |
| `--resume` | Продовжити перерваний процес |
| `--skipfirst` | Пропустити перший пакунок при resume |
| `--undo` | Скасувати останнє встановлення/видалення |
| `--clean-source-cache` | Очистити кеш VCS-джерел |
| `--list-sets` | Показати доступні `@sets` |
| `-h` / `--help` | Довідка |
| `-V` / `--version` | Версія |

Man-сторінка `emerge(1)` генерується з CLI через `clap_mangen` (`emerge --gen-manpage`), тож не розходиться з `--help`.

## Демонстрація

[vhs-emerge](repo-files/emerge.gif)

## Автор

**Undercat037**
