<div align="center">
  <img src="misc/logo/slash-logo-128x128.svg" width="128" alt="aura-emerge logo">

# Aura-Emerge Secure Package Manager

[en](README.MD) | ua
</div>

> Автономний ПМ у стилі Gentoo `emerge` для Arch Linux - встановлює з офіційних репозиторіїв, AUR та ABS, сканує PKGBUILD на ознаки атак на ланцюг постачання перед збіркою і виконує недовірені кроки збірки всередині пісочниці `bwrap`.

Керує пакунками через файл `world` - встановив один раз, відстежується назавжди.

## Встановлення

### AUR
```bash
paru -S aura-emerge
```
### Makepkg
```bash
git clone https://aur.archlinux.org/aura-emerge.git
cd aura-emerge
makepkg -si
```

<details>
<summary><h3>Ручна збірка/makepkg (Нестабільна)</h3></summary>

### Make
```bash
git clone https://github.com/Undercat037/aura-emerge.git
cd aura-emerge
make install
```
### Ручна збірка
```bash
git clone https://github.com/Undercat037/aura-emerge.git
cd aura-emerge
cargo build --release
sudo cp target/release/aura-emerge /usr/bin/emerge
sudo ln -sf /usr/bin/emerge /usr/bin/portageq
emerge --gen-completions bash | sudo tee /usr/share/bash-completion/completions/emerge
emerge --gen-completions zsh | sudo tee /usr/share/zsh/site-functions/_emerge
emerge --gen-completions fish | sudo tee /usr/share/fish/vendor_completions.d/emerge.fish
emerge --gen-manpage | sudo tee /usr/share/man/man1/emerge.1 >/dev/null
```
### Makepkg
```bash
git clone https://github.com/Undercat037/aura-emerge.git
cd aura-emerge
makepkg -si
```
</details>


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

[**→ Приклад makepkg.conf**](misc/makepkg.conf)

## make.conf

`/etc/portage/make.conf` (і `~/.config/emerge/make.conf` для власних налаштувань) - окремий конфіг самого aura-emerge, не `makepkg.conf`: аналог того, як Gentoo `make.conf` тримає і `EMERGE_DEFAULT_OPTS`, і `CFLAGS` в одному файлі. Спершу системний файл, потім користувацький; при збігу ключа перемагає останній.

```bash
sudo nvim /etc/portage/make.conf
```

```toml
# Прапори, які підставляються перед будь-яким запуском, до того, що
# набрано в командному рядку (той завжди перемагає при конфлікті).
# Вимкнути для одного запуску: --ignore-default-opts.
EMERGE_DEFAULT_OPTS = ["--pkgbuild-view", "--unshare-net-build"]

# Прапори збірки, застосовуються через згенерований makepkg.conf
# (makepkg --config) - перевизначають /etc/makepkg.conf, не чіпаючи його.
[build]
CFLAGS = "-march=native -O2 -pipe"
CXXFLAGS = "$CFLAGS"
RUSTFLAGS = "-C target-cpu=native"
MAKEFLAGS = "-j$(nproc)"
NINJAFLAGS = "-j$(nproc)"
OPTIONS = ["strip", "docs", "!debug", "lto"]
BUILDENV = ["!distcc", "color", "check", "!sign"]
```

Розпізнавані ключі: `EMERGE_DEFAULT_OPTS` на верхньому рівні, і в `[build]`: `CFLAGS`, `CXXFLAGS`, `CPPFLAGS`, `LDFLAGS`, `RUSTFLAGS`, `MAKEFLAGS`, `NINJAFLAGS`, `BUILDENV`, `OPTIONS`. Ключі збірки приймаються також на верхньому рівні, поза `[build]`.

Скаляри й списки взаємозамінні всюди - список склеюється пробілами, рядок розбивається по них. Значення розкриваються шеллом, коли makepkg читає згенерований конфіг, тож `-j$(nproc)` працює, і посилання на інший ключ вище по файлу теж - `CXXFLAGS = "$CFLAGS"`; TOML literal string (`'\$FOO'`) залишає долар буквальним.

`EMERGE_DEFAULT_OPTS` приймає лише прапори-модифікатори (allowlist: `--ask`, `--verbose`, `--devel`, `--tree`, `--searchdesc`, `--exclude=<атом>` і подібні - див. [приклад файлу](misc/make.conf)) - перетворити кожен запуск на `--unmerge` не вийде. Конфліктні пари (`--aur`/`--abs`, `--aur`/`--only-repos`, `--no-sandbox`/`--unshare-net-build`, `--skippgp`/`--autopgp`, `--select`/`--deselect`) відхиляються, якщо обидва прапори активні - набрані вручну чи з конфігу; прапор з конфігу, що конфліктує з набраним вручну, просто відкидається, бо командний рядок - конкретніший намір. `--abs` і `--only-repos` конфліктом **не є** - ABS збирає офіційний пакунок з його ж джерела, тож ці два узгоджені.

`--info` показує, що прочитано, і фактичне, розкрите шеллом значення кожного перевизначення.

[**→ Приклад make.conf**](misc/make.conf)

## Маска (/etc/portage/package.mask)

Аналог Portage `package.mask` для Arch-атомів: пакунки, які ця машина ніколи не встановлює - перевіряється навіть як транзитивна AUR-залежність і навіть при явному запиті за іменем. По одному запису на рядок у `/etc/portage/package.mask` або `/etc/portage/package.mask/*.mask`, дозволено `*`-глоб, опційний префікс репозиторію, опційна причина після `#`:

```
ayugram-desktop-bin            # конкретний пакунок, з будь-якого джерела
aur/*-bin                      # ніколи не ставити AUR -bin пакунки
*-git                          # devel-пакунки на цій машині не потрібні
extra/nano                     # конкретний пакунок з конкретного репозиторію
```

Замаскований пакунок - жорстка зупинка запиту, без запитання, без прапора для обходу. Відрізняється від `--exclude` нижче: маска - постійне рішення, `--exclude` діє лише в межах одного запуску.

## --exclude / --keep-going

`--exclude <атом>` (повторюваний, або через кому) виключає пакунок із поточного запуску - як `--exclude` у Portage. Діє при встановленні, `-u` (через `pacman --ignore`), провізіюванні `@world`, `--depclean`/`--prune` (захищає від видалення) та `--revdep-rebuild`.

```bash
emerge -u --exclude linux --exclude aur/ayugram-desktop-bin   # оновити все, крім цих
emerge --exclude firefox @world                               # провізіювання, пропустити firefox
```

`--keep-going` не зупиняє пакетну операцію (`@world`, `-u`, `--batchinstall`, встановлення кількох пакунків) на першій же помилці - продовжує, збирає, що не вдалося, і в кінці друкує підсумок з ненульовим кодом виходу. `--resume` після запуску з `--keep-going` повторює лише пакунки, що впали, а не весь пакет.

```bash
emerge -u --keep-going                # повне оновлення, не зупинятись на одному пакунку
emerge --batchinstall pkgs.txt --keep-going
emerge --resume                       # повторить лише те, що впало минулого разу
```

## -t/--tree, --deep

`-t`/`--tree` показує план (з `-p`/`--pretend`, або прямо перед реальним встановленням) як дерево залежностей замість плоского списку, у стилі `emerge -pt`. За замовчуванням один рівень вкладеності - те, що набрано, на глибині 0, а те, що воно затягнуло як залежність, з відступом під ним - атрибуція через звірку `pacman -Si`'s "Depends On" кожного запитаного пакунка з усім іншим у плані. Додайте `--deep`, щоб вкладати через стільки рівнів, скільки триває реальний ланцюг залежностей, або `--deep=N`, щоб обмежити N рівнями, а не зупинятись на прямих.

```bash
emerge -apt noctalia --aur     # ask + pretend + tree: перегляд перед збіркою
emerge -pt noctalia --aur --deep   # те саме, але вкладено до самого низу
emerge deepin-session --ask --tree --deep=3   # встановлення з показом до 3 рівнів дерева залежностей
```

```
[ebuild  N ] noctalia-5.1.0-1.1
[ebuild  N ] `-- libqalculate-5.12.0-1.1
[ebuild  N ]   `-- sdbus-cpp-2.3.1-1.1
```

Залежність, яку pacman затягнув, але яка буквально ні в чиєму "Depends On" не згадана (резолвнута через `provides`, або поза межами `--deep`) - все одно показується, просто без конкретного батька для вкладення: реальна інформація, а не вигаданий деревовидний вигляд. Для чистого `--aur`-встановлення крок побудови плану поки не резолвить транзитивні AUR-only залежності, тож `-t` там наразі плоский, так само як і без нього.

## Лог merge/unmerge

`/var/log/emerge.log` - аналог Portage `emerge.log`, для pacman/AUR/ABS-атомів. Append-only, по одному рядку на подію, пишеться так само, як і інший root-owned стан у `/etc/portage/*` (`sudo tee`, best-effort - невдалий запис у лог ніколи не заваляє merge/unmerge, який вже стався).

```
2026-09-21 08:51:44  MERGE    aur    noctalia-5.1.0-1.1        (12s)
2026-09-21 08:52:10  MERGE    aur    libqalculate-5.12.0-1.1   (9s)
2026-09-21 09:03:02  MERGE    repo   nano-8.0-1  vim-9.1-1     (4s)
2026-09-21 09:10:05  UNMERGE  -      old-package-2.0-1
```

Тривалість завжди справжня: AUR/ABS-пакунки збираються по одному в циклі самого інструмента, тож у кожного свій таймер; транзакція `pacman -S`/`-Syu` - це один код виходу на весь пакет, тож той рядок несе всі атоми в ньому і загальний час пакета, а не вигаданий рівний поділ. `--info` показує накопичену статистику (кількість merge/unmerge, сумарний час збірки).

## Використання

```bash
emerge neovim                              # офіційні репо, при відсутності - AUR
emerge neovim-git --aur                    # лише AUR (AUR-only залежності збираються рекурсивно завжди)
emerge ayugram-desktop-bin --aur --sudoloop   # те саме, але тримати sudo «теплим» протягом усієї рекурсивної збірки
emerge neovim --only-repos                 # лише офіційні репо, без AUR
emerge neovim --abs                        # зібрати з ABS (pkgctl), у пісочниці bwrap
emerge neovim --abs --no-sandbox           # ABS без пісочниці (звичайний makepkg -si)
emerge neovim --abs --skippgp              # ABS, пропустити перевірку PGP
emerge neovim --abs --autopgp              # ABS, автоімпорт відсутніх PGP-ключів
emerge neovim --abs --edit                 # ABS, редагувати PKGBUILD ($EDITOR), оновлює .SRCINFO
emerge neovim-git --aur --edit             # AUR, редагувати PKGBUILD, оновлює .SRCINFO
emerge neovim-git --aur --edit --skip-srcinfo-regen  # редагувати без оновлення .SRCINFO
emerge neovim-git --aur --unshare-net-build     # збірка без мережі на етапі build()/check()/package()
emerge -c                                  # видалити сиріт (depclean)
emerge -C neovim                           # видалити пакунок (unmerge)
emerge @world                              # provision: встановити відсутнє з world
emerge @world --err-install                   # також спробувати записи Err/ звичайним шляхом
emerge -u @world                           # повне оновлення (репо + AUR); world не читається
emerge -u @world --refresh                 # оновлення з примусовим -Syy
emerge --sync                              # синхронізувати бази
emerge @game-kit                           # власний набір з /etc/portage/sets/game-kit.set
emerge @game-kit neovim                    # набори + звичайні пакунки
emerge --list-sets                         # показати доступні @sets
emerge --info                              # інформація про систему (як Gentoo emerge --info)
emerge --news                              # новини Arch Linux, непрочитані позначені
emerge --news 3                            # прочитати новину 3
emerge --news all                          # позначити всі як прочитані
emerge neovim-git --aur --edit --pkgbuild-view  # перегляд/diff PKGBUILD поверх --edit
emerge neovim --abs --pkgbuild-view        # те саме для ABS
emerge --install-pkgbuild ~/src/my-pkg        # зібрати+встановити локальний checkout PKGBUILD (сканер + bwrap, як AUR/ABS)
emerge --install-pkgbuild ~/src/my-pkg --pkgbuild-view --skippgp
emerge --scan neovim-git --aur              # лише аудит, без збірки
emerge --install-pkgbuild ~/src/my-pkg --scan  # те саме для локального checkout
emerge -u --devel                           # повне оновлення + перевірка upstream для -git/-hg/-svn/-bzr
emerge --check-devel                        # лише звіт, які devel-пакунки зрушили upstream
emerge -s neovim                           # пошук (репо + AUR)
emerge -s neovim --only-repos              # пошук лише в офіційних репо
emerge --searchdesc neovim                 # пошук за описом
emerge -sv neovim                          # детальна інформація
emerge -sv neovim-git --aur                # детальна інформація лише з AUR
emerge -1 htop                             # як залежність (не в world)
emerge -n neovim                           # не перевстановлювати (--noreplace)
emerge -p neovim                           # dry run
emerge -a neovim                           # запитати перед дією
emerge --select neovim                     # додати до world без встановлення
emerge --deselect neovim                   # прибрати з world без видалення
emerge --prune                             # видалити пакунки поза world
emerge --regen                             # оновити кеш метаданих
emerge --regen-world                       # оновити префікси репо у world
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
emerge --batchinstall pkgs.txt              # масове встановлення зі списку
emerge --batchinstall pkgs.txt --aur -a     # поєднується з будь-яким іншим прапором
emerge -u --exclude linux                   # оновити, притримавши один пакунок
emerge -u --keep-going                      # оновити, не зупинятись на першій помилці
emerge --revdep-rebuild                     # знайти й полагодити биті лінки бібліотек
emerge --revdep-rebuild -p                  # те саме, лише список
```

## Пісочниця

Збірки **AUR і ABS** виконують функції PKGBUILD `pkgver()` / `prepare()` / `build()` / `check()` / `package()` всередині простору імен [`bwrap`](https://github.com/containers/bubblewrap) (якщо встановлено `bubblewrap`):

- фейковий `$HOME` (немає SSH-ключів, GPG secret keyring, профілю браузера);
- запис лише в директорію збірки (+ налаштовані `PKGDEST` / `SRCDEST` / `SRCPKGDEST` / `BUILDDIR`);
- мережа спільна за замовчуванням (`--share-net`);
- решта просторів імен (user / IPC / PID / UTS / cgroup) знищена;
- `fakeroot` для `package()` працює у власному вкладеному `bwrap` з uid 0 (лагодить `fchownat` у звичайному user namespace); скрипт-шим для нього лежить у `/var/tmp`, поза директорією збірки, щоб `prepare()`/`build()` не могли підмінити його до запуску `package()`.

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

## Масове встановлення

`--batchinstall <FILE>` читає текстовий список пакунків і додає його до звичайного списку до будь-яких дій - той самий формат, що й [власний набір](#власні-набори-sets) (один атом на рядок, `#`-коментарі дозволені), але з довільного шляху, без реєстрації в `/etc/portage/sets/`.

```
# pkgs.txt
neovim
aur/visual-studio-code-bin
firefox
```

```bash
emerge --batchinstall pkgs.txt
```

Оскільки список зливається з `cli.packages`, працює з усіма іншими прапорами так само, як іменовані пакунки (`-p`, `-a`, `--aur`, `--abs`, `--only-repos`, ...).

## Перегляд PKGBUILD

`--pkgbuild-view` показує PKGBUILD перед збіркою і просить підтвердження - diff відносно останньої показаної копії цього pkgbase (кеш `~/.cache/aura-emerge/pkgbuild-view/`), або повний файл уперше. Відмова пропонує відкрити його в `$EDITOR` замість простого пропуску. Поєднується з `--edit` (який іде першим, тож view/diff відображає щойно зроблені правки). Працює для `--aur`/звичайного AUR-fallback і `--abs`, лише для безпосередньо запрошеного пакунка - ніколи для рекурсивно підтягнутої AUR-залежності і ніколи для пакетних шляхів (`@world`, `-u`, `--undo`).

```bash
emerge neovim-git --aur --pkgbuild-view
emerge neovim --abs --edit --pkgbuild-view   # спочатку edit, потім view/diff результату
```

`--install-pkgbuild <PATH>` збирає і встановлює довільний локальний checkout PKGBUILD через той самий пайплайн, що й AUR/ABS - сканер, потім пісочниця bwrap - замість «голого» `makepkg -si`. У `PATH` має бути `PKGBUILD`; якщо немає `.SRCINFO`, він генерується (`makepkg --printsrcinfo`, як після `--edit`), щоб залежності розв’язувались статично. Поєднується з `--pkgbuild-view`, `-a`/`--ask`, `-1`/`--oneshot`, `--skippgp`, `--no-sandbox`, `--unshare-net-build`, `--skip-srcinfo-regen`; несумісний з іменами пакунків/`@set` або `--aur`/`--abs`. У `world` записується з префіксом `Err/` (джерело невідоме - див. [world](#worldset)), якщо не задано `-1`/`--oneshot`.

```bash
emerge --install-pkgbuild ~/src/my-pkg
emerge --install-pkgbuild ~/src/my-pkg --pkgbuild-view --skippgp
```

`--scan` перевіряє PKGBUILD (+ `.install`) без збірки й встановлення - тягне з cgit для імені AUR-пакунка або читає локальний checkout з `--install-pkgbuild <PATH> --scan`. Той самий звіт, що й звичайний pre-build scan, без запиту `Continue anyway?` (немає куди продовжувати). Ненульовий код виходу при будь-якій знахідці - зручно в скриптах. Поки що не підключено до `--abs`.

```bash
emerge --scan neovim-git --aur
emerge --install-pkgbuild ~/src/my-pkg --scan
```

## Перевірка upstream devel-пакунків

Конвенція Arch для `-git`/`-hg`/`-svn`/`-bzr` обчислює версію в `pkgver()` з історії VCS під час збірки - але версія *на сторінці AUR* змінюється лише коли мейнтейнер вручну перепушує PKGBUILD, і для активного пакунка це може відставати від реального upstream на тижні чи місяці. Звичайний `-u` порівнює лише записану AUR-версію, тож часто ніколи не пропонує перезбірку, навіть коли upstream уже пішов далі.

`--devel` (разом із `-u`) і `--check-devel` (окремо, лише звіт) закривають цю дірку: для кожного встановленого devel-пакунка парсять `git+` URL прямо з PKGBUILD і роблять `git ls-remote` проти нього - без локального клону - порівнюючи з останнім баченим хешем (`~/.cache/aura-emerge/devel.state`). Поки що лише git; `-hg`/`-svn`/`-bzr` розпізнаються, але звітуються як «не вдалося визначити», а не мовчки пропускаються.

```bash
emerge -u --devel        # звичайне повне оновлення + перезбірка devel, чий upstream зрушив
emerge --check-devel     # лише звіт, хто зрушив, без перезбірки
```

## @preserved-rebuild

В Arch немає відстеження preserved-libs. `@preserved-rebuild` збирає залежності кожного встановленого пакунка (`pacman -Qi`), передає список у `pacman -T` і пропонує перевстановити незадоволені через `pacman -S --asdeps` (щоб вони не виглядали «явно встановленими» і не потрапили в world при `--regen-world-from-explicit`).

## --revdep-rebuild

Не те саме, що `@preserved-rebuild` вище - та перевірка бачить лише *оголошені* залежності через `pacman -T`. `--revdep-rebuild` читає встановлені ELF-файли напряму (`DT_NEEDED`, `DT_RPATH`/`DT_RUNPATH`) і знаходить бінарники, що лінкуються на `.so`, якого вже немає - класична поломка після оновлення `openssl`/`icu`/`boost`, яку Gentoo вирішує через preserved-libs, а Arch із коробки - ні.

Foreign-пакунки (AUR/локальні) пересобираються звичайним шляхом через пісочницю; офіційні пакунки на бінарному дистрибутиві пересобрати не можна, тож для них бібліотеки, яких не вистачає, шукаються у файловій базі (`pacman -F`), і пропонується встановити пакунки, що їх надають. `--exclude` і [маска](#маска-etcemergemask) також діють тут.

```bash
emerge --revdep-rebuild       # сканувати, потім запитати перед виправленням
emerge --revdep-rebuild -p    # лише сканувати, показати биті лінки
```

## world

Файл: `/etc/portage/world`. Кожен явно встановлений пакунок додається автоматично, при видаленні - прибирається. Можна редагувати вручну (один пакунок на рядок).

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

- **`emerge @world` (без `-u`)** - *provision*. Встановлює те, чого немає з `world`; уже встановлене не чіпається. Префікс визначає джерело. Зручно тримати `world` у dotfiles і на новій машині зробити `emerge @world`.
- **`emerge -u @world`** (або `emerge -u`) - *upgrade*. Оновлює все встановлене (репо + AUR). `world` не читається.
- **`emerge --sync`** - лише освіження баз (`-Sy` / `-Syy`).

## Власні набори (sets)

`/etc/portage/sets/<name>.set` викликається як `@<name>`:

```
# /etc/portage/sets/game-kit.set
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

1. **AST** (`tree-sitter-bash`) - `curl|sh`, `base64`/`xxd | sh`, `source <(curl …)`, `eval "$(curl …)"`, `sudo`/`pkexec`/`doas`, python inline-exec, `sh`/`bash`/`dash`/`ash` `-c` з підозрілим payload, top-level command substitution (`` `...` `` / `$(...)` поза тілами функцій - напр. markdown-бектейки в `pkgdesc`) тощо. Тривіальні текстові обходи (конкатенація рядків, абсолютні шляхи, `env`) не проходять.
2. **Текстові евристики** - `chmod 777`, «сирі» IP, сторонні пакетні менеджери (`npm install …`), paste-сайти, `.onion`, зонди anti-sandbox/anti-debugger (`TracerPid`, `LD_PRELOAD`/`LD_LIBRARY_PATH`), відомі IOC (Atomic Arch, openconnect-sso, `compromised.txt` з 2018 тощо).

Знахідки: `Suspicious` або `ConfirmedIoc`. Спрацювання не блокує жорстко - показує файл/рядок, посилання на cgit, запит `Continue anyway? [y/N]`.

Збірки AUR і ABS додатково захищені [пісочницею `bwrap`](#пісочниця).

<details>
<summary><h3>Тестування(після встановлення)</h3></summary>

```bash
git clone https://github.com/Undercat037/aura-emerge.git
cd aura-emerge
emerge --install-pkgbuild ./scaner-test --ask
```
</details>

<details>
<summary><h2>Прапори</h2></summary>

| Прапор | Опис |
|--------|------|
| `-s` | Пошук пакунка |
| `--searchdesc` | Пошук за описом |
| `-sv` | Детальна інформація |
| `-u` | Повне оновлення системи (world не читається) |
| `--devel` | Разом із `-u` - також перезібрати `-git`/`-hg`/`-svn`/`-bzr`, чий upstream зрушив |
| `--check-devel` | Звіт, які devel-пакунки мають зміни upstream (без перезбірки) |
| `-C` | Видалити пакунки (unmerge) |
| `-c` | Видалити сиріт (depclean) |
| `-p` | Симуляція (dry run) |
| `-a` | Запит підтвердження |
| `--sudoloop` | Тримати sudo timestamp «теплим» протягом усього запуску |
| `-1` | Встановити як залежність (oneshot) |
| `-v` | Детальний вивід |
| `-n` | Не перевстановлювати (noreplace) |
| `--aur` | Примусово AUR (AUR-only залежності завжди резолвляться і збираються рекурсивно) |
| `--only-repos` | Лише офіційні репо |
| `--abs` | Зібрати з ABS (`pkgctl`), у пісочниці bwrap |
| `--no-sandbox` | Вимкнути bwrap для AUR/ABS |
| `--unshare-net-build` | Без мережі на етапі build()/check()/package() |
| `--skippgp` | Пропустити перевірку PGP (ABS) |
| `--autopgp` | Автоімпорт відсутніх PGP-ключів (ABS) |
| `--edit` | Редагувати PKGBUILD перед збіркою; після збереження оновлює `.SRCINFO` |
| `--pkgbuild-view` | Показати/diff PKGBUILD, підтвердити перед збіркою (при відмові пропонує `$EDITOR`) |
| `--install-pkgbuild <PATH>` | Зібрати+встановити локальний checkout PKGBUILD (сканер + bwrap) |
| `--scan` | Лише аудит PKGBUILD/.install, без збірки/встановлення (AUR або `--install-pkgbuild`) |
| `--skip-srcinfo-regen` | Разом з `--edit` - не оновлювати `.SRCINFO` |
| `--sync` | Синхронізувати бази |
| `--refresh` | Примусово `-Syy` (з `--sync` або `-u`) |
| `--info` | Інформація про систему |
| `--news [N\|all]` | Новини Arch Linux |
| `--check-news [N\|all]` | Синонім `--news` |
| `--select` | Додати до world без встановлення |
| `--deselect` | Прибрати з world без видалення |
| `--prune` | Видалити пакунки поза world |
| `--regen` | Оновити кеш метаданих |
| `--regen-world` | Оновити префікси репо у world |
| `--regen-sets <SET>` | Оновити префікси для одного набору |
| `--regen-sort` | Разом з `--regen-sets` - відсортувати файл |
| `--regen-world-from-explicit` | Міграція: наповнити world з `pacman -Qeq` |
| `--err-install` | При `@world` також пробувати записи `Err/` |
| `--resume` | Продовжити перерваний процес |
| `--skipfirst` | Пропустити перший пакунок при resume |
| `--undo` | Скасувати останнє встановлення/видалення |
| `--clean-source-cache` | Очистити кеш VCS-джерел |
| `--batchinstall <FILE>` | Масове встановлення зі списку пакунків |
| `--list-sets` | Показати доступні `@sets` |
| `--exclude <ATOM>` | Виключити пакунок з цього запуску (повторюваний/через кому) |
| `--keep-going` | Не зупинятись на першій помилці в пакетній операції; підсумок в кінці |
| `--revdep-rebuild` | Знайти й полагодити бінарники з бітими лінками бібліотек |
| `-t`, `--tree` | Показати план як дерево залежностей |
| `-D`, `--deep[=N]` | З `-t`: вкладати на N рівнів, або на всі - без числа |
| `--ignore-default-opts` | Ігнорувати `EMERGE_DEFAULT_OPTS` з make.conf для цього запуску |
| `-h` / `--help` | Довідка |
| `-V` / `--version` | Версія |

Man-сторінка `emerge(1)` генерується з CLI через `clap_mangen` (`emerge --gen-manpage`), тож не розходиться з `--help`.
</details>

## Автор/Ліцензія
Copyright (C) 2026 **Undercat037**
GPL-3.0 license