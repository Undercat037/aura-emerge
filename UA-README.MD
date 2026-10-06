<div align="center">
  <img src="misc/logo/slash-logo-128x128.svg" width="128" alt="aura-emerge logo">

# Aura-Emerge Secure Package Manager

[![AE_Site](https://img.shields.io/badge/AE_Site-333333?style=flat-square&logo=bookstack&logoColor=FFFFFF)](https://undercat037.github.io/aura-emerge/)
[![AUR](https://img.shields.io/badge/AUR-333333?style=flat-square&logo=arch-linux&logoColor=1793D1)](https://aur.archlinux.org/packages/aura-emerge)
[![GitHub](https://img.shields.io/badge/GitHub-333333?style=flat-square&logo=github&logoColor=FFFFFF)](https://github.com/Undercat037/aura-emerge)
[![GitLab](https://img.shields.io/badge/GitLab-333333?style=flat-square&logo=gitlab&logoColor=FC6D26)](https://gitlab.com/Undercat037/aura-emerge)

[en](README.MD) | ua | [ru](RU-README.MD)
</div>

> Автономний менеджер пакунків `emerge` у стилі Gentoo для Arch Linux. Встановлює з офіційних репозиторіїв, AUR та ABS; перед збіркою сканує PKGBUILD на ознаки атак на ланцюг постачання; недовірені кроки збірки виконує в пісочниці `bwrap`.

Пакунки обліковуються у файлі `world` - встановив один раз, відстежується назавжди.

## Встановлення

```bash
paru -S aura-emerge          # через AUR-хелпер
```

або звичайним makepkg:

```bash
git clone https://aur.archlinux.org/aura-emerge.git
cd aura-emerge
makepkg -si
```

<details>
<summary><h3>Збірка з вихідного коду (нестабільно)</h3></summary>

```bash
git clone https://github.com/Undercat037/aura-emerge.git
cd aura-emerge
make install            # або вручну:
```

```bash
cargo build --release
sudo cp target/release/aura-emerge /usr/bin/emerge
sudo ln -sf /usr/bin/emerge /usr/bin/portageq
emerge --gen-completions bash | sudo tee /usr/share/bash-completion/completions/emerge
emerge --gen-completions zsh  | sudo tee /usr/share/zsh/site-functions/_emerge
emerge --gen-completions fish | sudo tee /usr/share/fish/vendor_completions.d/emerge.fish
emerge --gen-manpage | sudo tee /usr/share/man/man1/emerge.1 >/dev/null
```
</details>

**Залежності:** `git`, `bubblewrap` (пісочниця збірки для AUR/ABS)
**Опційно:** `devtools` (підтримка `--abs`), `gnupg` (`--autopgp`, автоімпорт PGP-ключів)

## Використання

```bash
# Встановлення
emerge neovim                       # офіційні репозиторії, у разі невдачі - AUR
emerge neovim-git --aur             # лише AUR (залежності лише з AUR збираються рекурсивно)
emerge neovim --repos               # лише офіційні репозиторії, AUR не чіпати
emerge neovim --abs                 # збірка з ABS (pkgctl) у пісочниці
emerge -p neovim                    # пробний прогін
emerge -a neovim                    # запитати перед дією
emerge -1 htop                      # встановити як залежність (у world не потрапляє)

# Видалення
emerge -C neovim                    # видалити пакунок (unmerge)
emerge -c                           # видалити сиріт (depclean)
emerge --prune                      # видалити пакунки, яких немає у world

# Оновлення
emerge --sync                       # оновити бази пакунків
emerge -u                           # повне оновлення (репозиторії + AUR)
emerge -u --devel                   # ...і перезібрати -git/-hg/-svn/-bzr, чий upstream пішов уперед
emerge -u --exclude linux --keep-going

# World і набори
emerge @world                       # встановити все зі world, чого ще немає
emerge @game-kit neovim             # власний набір + звичайний пакунок
emerge --select neovim              # додати у world без встановлення
emerge --deselect neovim            # прибрати зі world без видалення

# Пошук та інформація
emerge -s neovim                    # пошук: репозиторії, потім ABS, потім AUR
emerge -S neovim                    # пошук за описом
emerge -sv neovim                   # докладна інформація про пакунок
emerge --info                       # інформація про систему
emerge --news                       # новини Arch Linux

# Безпека
emerge neovim-git --aur --pkgbuild-view      # показати/порівняти PKGBUILD перед збіркою
emerge neovim-git --aur --edit               # спершу відредагувати PKGBUILD
emerge --scan neovim-git --aur               # лише аудит, без збірки
emerge --install-pkgbuild ~/src/my-pkg       # зібрати локальний PKGBUILD через сканер і пісочницю

# Відновлення
emerge --resume                     # продовжити перервану транзакцію
emerge --undo                       # скасувати останнє встановлення/видалення
emerge --revdep-rebuild             # полагодити бінарники зі зламаними посиланнями на бібліотеки
```

Повна таблиця прапорів - наприкінці сторінки.

## Налаштування

### Gentoo → Arch: аналоги `make.conf`

В Arch немає USE-прапорів, але компіляцію можна налаштувати майже так само, як у `/etc/portage/make.conf` у [Gentoo](https://wiki.gentoo.org/wiki/Main_Page) - через `/etc/makepkg.conf`:

| Gentoo (`make.conf`)         | Arch (`makepkg.conf`)         | Призначення                      |
|------------------------------|-------------------------------|----------------------------------|
| `CFLAGS="-O2 -march=native"` | `CFLAGS="-O2 -march=native"`  | Прапори компілятора C            |
| `CXXFLAGS="-O2 -march=..."`  | `CXXFLAGS="-O2 -march=..."`   | Прапори компілятора C++          |
| `RUSTFLAGS="-C opt-level=3"` | `RUSTFLAGS="-C opt-level=3"`  | Прапори компілятора Rust         |
| `MAKEOPTS="-j$(nproc)"`      | `MAKEFLAGS="-j$(nproc)"`      | Паралельні завдання збірки       |
| `USE="lto"`                  | `OPTIONS+=(lto)`              | Оптимізація на етапі компонування |
| `FEATURES="ccache"`          | `BUILDENV+=(ccache)`          | Кеш збірки                       |
| `PORTDIR` / `PKGDIR`         | `PKGDEST`, `SRCDEST`          | Каталоги артефактів збірки       |

Див. [прапори оптимізації GCC](https://gcc.gnu.org/onlinedocs/gcc/Optimize-Options.html), [параметри кодогенерації rustc](https://doc.rust-lang.org/rustc/codegen-options/index.html) та [**приклад makepkg.conf**](misc/makepkg.conf).

### `/etc/portage/make.conf`

Власний конфіг aura-emerge. Перевизначає `/etc/makepkg.conf` для збірок через aura-emerge, не чіпаючи сам файл.

```bash
# Прапори за замовчуванням, підставляються перед набраним вами (командний рядок головніший).
# Вимкнути для одного запуску: --ignore-default-opts.
EMERGE_DEFAULT_OPTS="--pkgbuild-view --unshare-net-build"

# Прапори збірки (застосовуються через згенерований makepkg.conf)
CFLAGS="-march=native -O2 -pipe"
CXXFLAGS="$CFLAGS"
RUSTFLAGS="-C target-cpu=native"
MAKEFLAGS="-j$(nproc)"
NINJAFLAGS="-j$(nproc)"
OPTIONS=(strip docs !debug lto)
BUILDENV=(!distcc color check !sign)

# Перемикачі в стилі Portage, перекладаються в makepkg
FEATURES="ccache -test"
```

**Ключі:** `EMERGE_DEFAULT_OPTS`, `FEATURES`, `CFLAGS`, `CXXFLAGS`, `CPPFLAGS`, `LDFLAGS`, `RUSTFLAGS`, `MAKEFLAGS`, `NINJAFLAGS`, `BUILDENV`, `OPTIONS`.
**Синтаксис:** як у bash - `KEY="..."` або `KEY=(a b c)`. Рядки та списки взаємозамінні, значення розкриваються оболонкою (`$(nproc)` і `$CFLAGS` працюють; одинарні лапки залишать `$` як є).

Токени `FEATURES` (`-name` вимикає токен) перекладаються в аналоги makepkg. Те, що ви самі вказали в `BUILDENV`/`OPTIONS`, головніше.

| `FEATURES`   | makepkg             |
|--------------|---------------------|
| `ccache`     | `BUILDENV` `ccache` |
| `distcc`     | `BUILDENV` `distcc` |
| `test`       | `BUILDENV` `check`  |
| `nostrip`    | `OPTIONS` `!strip`  |
| `splitdebug` | `OPTIONS` `debug`   |

`EMERGE_DEFAULT_OPTS` приймає лише прапори-модифікатори (`--ask`, `--verbose`, `--devel`, `--tree`, `--searchdesc`, `--exclude=<atom>`, ...), тож перетворити кожен запуск на `--unmerge` не можна. Конфліктні пари відхиляються, якщо задано обидва: `--aur`/`--abs`, `--aur`/`--repos`, `--no-sandbox`/`--unshare-net-build`, `--skippgp`/`--autopgp`, `--select`/`--deselect`. Якщо значення з конфігу конфліктує з командним рядком, воно відкидається. `--abs` і `--repos` **не** конфліктують.

`emerge --info` показує, що було прочитано, і підсумкове значення кожного перевизначення. [**→ Приклад make.conf**](misc/make.conf)

### `package.mask`

Пакунки, які ця машина не повинна встановлювати ніколи: перевірка спрацьовує навіть для транзитивних залежностей з AUR і навіть коли пакунок запитано за іменем. Замаскований пакунок жорстко зупиняє запит - без питань і без прапора для обходу.

`/etc/portage/package.mask` - файл або каталог (читається кожен звичайний файл у порядку імен; файли з крапки пропускаються; символьні посилання відхиляються). Один запис на рядок, дозволено `*` і префікс репозиторію, наприкінці можна додати `# коментар`.

```
ayugram-desktop-bin            # конкретний пакунок, з будь-якого джерела
aur/*-bin                      # ніколи не встановлювати -bin пакунки з AUR
*-git                          # на цій машині без devel-пакунків
extra/nano                     # конкретний пакунок з конкретного репозиторію
```

Це не те саме, що `--exclude`: маска діє постійно, `--exclude` - на один запуск.

### `package.env`

Прапори збірки для окремих пакунків. Кожен запис - атом і одне або кілька імен env-файлів із `/etc/portage/env/`. Як і `package.mask`, `/etc/portage/package.env` може бути файлом або каталогом.

```
# /etc/portage/package.env/neovim
abs/neovim  lto.conf            # префікс репозиторію і '*' працюють як у package.mask
aur/*-git   no-lto  fast        # кілька env, застосовуються в указаному порядку
```

```bash
# /etc/portage/env/lto.conf  (синтаксис make.conf; враховуються лише ключі збірки)
CFLAGS="-march=znver3 -O2 -pipe"
MAKEFLAGS="-j16"
OPTIONS="strip docs !debug lto"
```

- **Накладання:** `make.conf` → відповідні записи `package.env` → кожен env-файл у вказаному порядку. Перемагає шар, який задав ключ останнім, і ключ замінюється повністю, а не зливається (`OPTIONS` з env-файлу замінює `OPTIONS` з `make.conf`). Збірка напише, який шар переміг.
- **Збіг:** за pkgbase або будь-яким із pkgname.
- **Область:** лише збірки AUR/ABS. `EMERGE_DEFAULT_OPTS` в env-файлі ігнорується з попередженням.
- **Помилки:** символьне посилання, некоректний запис або відсутній env-файл пропускаються з попередженням (з файлом і рядком); решта застосовується.

## Можливості

### `--exclude` і `--keep-going`

`--exclude <atom>` (можна повторювати або перелічувати через кому) виключає пакунок із поточного запуску: встановлення, `-u`, `@world`, `--depclean`/`--prune` (захищає від видалення) і `--revdep-rebuild`.

`--keep-going` не зупиняє пакетну операцію (`@world`, `-u`, `--batchinstall`, встановлення кількох пакунків) на першій помилці. Він запам'ятовує збої, наприкінці друкує підсумок і завершується з ненульовим кодом. Наступний `--resume` повторить лише те, що впало.

```bash
emerge -u --exclude linux --exclude aur/ayugram-desktop-bin   # оновити все, крім цих
emerge -u --keep-going                                        # не зупинятися на одному поганому пакунку з AUR
emerge --resume                                               # повторити лише те, що впало
```

### `-t` / `--tree` і `--deep`

Показує план (з `-p` або прямо перед справжнім встановленням) у вигляді дерева залежностей. За замовчуванням вкладено один рівень; `--deep` іде до самого низу, `--deep=N` - до N рівнів.

```bash
emerge -apt noctalia --aur           # ask + pretend + tree
emerge -pt noctalia --aur --deep     # на всю глибину
```

```
[ebuild  N ] noctalia-5.1.0-1.1
[ebuild  N ] `-- libqalculate-5.12.0-1.1
[ebuild  N ]   `-- sdbus-cpp-2.3.1-1.1
```

Залежність, яку не названо в чиємусь «Depends On» (розв'язано через `provides` або вона глибше за `--deep`), усе одно показується, просто без батька. Для чистого встановлення через `--aur` транзитивні залежності лише з AUR на кроці плану поки не розв'язуються, тож дерево там пласке.

### Журнал merge

`/var/log/emerge.log` - лише дозапис, один рядок на подію. Невдалий запис у журнал ніколи не ламає сам merge.

```
2026-09-21 08:51:44  MERGE    aur    noctalia-5.1.0-1.1        (12s)
2026-09-21 09:03:02  MERGE    repo   nano-8.0-1  vim-9.1-1     (4s)
2026-09-21 09:10:05  UNMERGE  -      old-package-2.0-1
```

Пакунки AUR/ABS збираються по одному, тому в кожного свій таймер. `pacman -S`/`-Syu` - одна транзакція, тож її рядок містить усі атоми та загальний час. `--info` показує підсумки (кількість merge/unmerge, сумарний час збірки).

### Масове встановлення

`--batchinstall <FILE>` встановлює пакунки з текстового списку (один атом на рядок, коментарі `#` дозволені). Формат той самий, що й у [власного набору](#власні-набори-sets), але файл можна взяти звідки завгодно - реєструвати не потрібно. Поєднується з будь-якими прапорами (`-p`, `-a`, `--aur`, `--abs`, `--repos`, ...).

```bash
emerge --batchinstall pkgs.txt
```

### Перегляд і перевірка PKGBUILD

**`--pkgbuild-view`** показує PKGBUILD прямо перед збіркою і просить підтвердження: diff з останньою показаною копією (кеш у `~/.cache/aura-emerge/pkgbuild-view/`) або весь файл уперше. У разі відмови пропонує відкрити `$EDITOR`, а не просто пропускає пакунок. Працює після `--edit`, тож ви бачите власні правки. Застосовується лише до напряму запитаних пакунків AUR/ABS - не до рекурсивних залежностей з AUR і не до пакетних шляхів (`@world`, `-u`, `--undo`).

**`--install-pkgbuild <PATH>`** збирає й встановлює локальний каталог із PKGBUILD через той самий конвеєр, що AUR/ABS (сканер → пісочниця), а не голим `makepkg -si`. У `PATH` має лежати `PKGBUILD`; відсутній `.SRCINFO` створюється. Поєднується з `--pkgbuild-view`, `-a`, `-1`, `--skippgp`, `--no-sandbox`, `--unshare-net-build` і `--skip-srcinfo-regen`, але не з пакунками/`@наборами` та `--aur`/`--abs`. У `world` пакунок записується як `Err/` (джерело невідоме), якщо не вказано `-1`.

**`--scan`** перевіряє PKGBUILD (+ `.install`), нічого не збираючи й не встановлюючи: для пакунка з AUR бере файли з cgit, для локального каталогу - `--install-pkgbuild <PATH> --scan`. Звіт той самий, що й при звичайному скануванні перед збіркою, лише без питання «Continue anyway?». Завершується з ненульовим кодом при будь-якій знахідці, тож годиться для скриптів. Для `--abs` поки недоступний.

```bash
emerge neovim-git --aur --pkgbuild-view
emerge --install-pkgbuild ~/src/my-pkg --pkgbuild-view --skippgp
emerge neovim-git --aur --scan
```

### Devel-пакунки (`-git`, `-hg`, `-svn`, `-bzr`)

Версія на сторінці AUR змінюється лише коли супровідник перезаливає пакунок, тому звичайний `-u` часто не помічає змін в upstream devel-пакунків.

`-u --devel` (оновлення) і `--check-devel` (лише звіт) закривають цю прогалину: для кожного встановленого devel-пакунка беруть URL `git+` з PKGBUILD, роблять `git ls-remote` (клон не потрібен) і порівнюють з хешем з минулого разу (`~/.cache/aura-emerge/devel.state`). Поки що лише git; `-hg`/`-svn`/`-bzr` виводяться як «couldn't determine», а не пропускаються мовчки.

```bash
emerge -u --devel        # оновити + перезібрати devel-пакунки, чий upstream пішов уперед
emerge --check-devel     # лише звіт
```

### Кеш джерел

Для пакунків `-git`/`-hg`/`-svn` `SRCDEST` за замовчуванням - `~/.cache/aura-emerge/sources` (або `$XDG_CACHE_HOME/aura-emerge/sources`), тож makepkg робить інкрементальний fetch замість повного клонування. Якщо `SRCDEST` задано в `makepkg.conf`, використовується він. Очистити кеш: `emerge --clean-source-cache`.

### `@preserved-rebuild` і `--revdep-rebuild`

В Arch немає обліку preserved-libs. Прогалину закривають два інструменти:

- **`@preserved-rebuild`** збирає *оголошені* залежності всіх встановлених пакунків (`pacman -Qi`), перевіряє їх через `pacman -T` і пропонує перевстановити відсутні з `--asdeps` (щоб їх не затягнуло у world подальшим `--regen-world-from-explicit`).
- **`--revdep-rebuild`** читає встановлені ELF-файли (`DT_NEEDED`, `DT_RPATH`/`DT_RUNPATH`) і знаходить бінарники, що лінкуються з неіснуючим уже `.so` - класична поломка після оновлення `openssl`/`icu`/`boost`. Пакунки AUR/локальні перезбираються через пісочницю; пакунки офіційних репозиторіїв тут перезібрати не можна, тож відсутня бібліотека шукається через `pacman -F`, а пакунок, що її надає, пропонується до встановлення. `--exclude` і [маска](#packagemask) діють.

```bash
emerge @preserved-rebuild -p     # лише список
emerge --revdep-rebuild          # просканувати, потім запитати перед виправленням
emerge --revdep-rebuild -p       # лише скан
```

### `world`

`/etc/portage/world` - список усіх явно встановлених пакунків. Записи додаються при встановленні, прибираються при видаленні й редагуються вручну (один пакунок на рядок).

```
core/nano
extra/firefox
aur/ayugram-desktop-bin
abs/nano
chaotic-aur/visual-studio-code-bin
```

**Префікси:** назва офіційного репозиторію, `aur/`, `abs/`, `Err/` (джерело невідоме).

> **Про `Err/`:** використовується для локальної збірки без відстежуваного походження та для запису `abs/`/`aur/`, який перестав розв'язуватися. `--regen-world` і `--regen-sets` зберігають відомий префікс `abs/`/`aur/`, а не відкочуються до `Err/`.

Якщо в пакетному встановленні одного імені немає, решта все одно встановлюється; відсутні виводяться наприкінці з ненульовим кодом виходу.

**`@world` має два значення:**

- `emerge @world` - *підготовка*: встановлює те, що є у `world`, але ще не встановлено. Тримайте `world` у dotfiles, клонуйте на новій машині, запускайте `emerge @world`.
- `emerge -u` (або `emerge -u @world`) - *оновлення*: оновлює все встановлене (репозиторії + AUR). `world` не читає.
- `emerge --sync` - лише оновлення баз (`-Sy` / `-Syy`).

### Власні набори (sets)

`/etc/portage/sets/<name>` (або `<name>.set`) викликається як `@<name>`. Один пакунок на рядок, коментарі `#` дозволені.

```
# /etc/portage/sets/game-kit
steam
lutris
gamemode
aur/protonup-qt
```

```bash
emerge @game-kit @dev-tools neovim    # набори вільно комбінуються
emerge --list-sets                    # список наборів (також: emerge @<TAB>)
emerge -s @game-kit                   # звідки береться кожен пакунок: репо / aur / not found
```

Якщо існують і `game-kit`, і `game-kit.set`, `@game-kit` відхиляється як неоднозначний. `@world` і `@preserved-rebuild` - це дії, тож `-s @world` і `-s @preserved-rebuild` дають помилку.

### `--undo`

Відкочує останнє успішне встановлення або видалення (один крок історії):

- **після встановлення** - видаляє лише пакунки, що в цьому запуску були новими;
- **після видалення** - перевстановлює із записаного джерела (`Err/`/`abs/` лише перелічуються);
- **після `-u`** - не підтримується (немає знімка версій).

### Новини

`emerge --news` отримує [RSS-стрічку новин Arch Linux](https://archlinux.org/feeds/news/). Стан прочитаного зберігається в `~/.cache/aura-emerge/news.state` (root не потрібен).

```bash
emerge --news        # список, непрочитані позначені [N]
emerge --news 3      # прочитати запис 3
emerge --news all    # позначити все прочитаним
```

## Безпека

### Сканування PKGBUILD

Перед кожним встановленням з AUR вихідний `PKGBUILD` (і будь-який хук `.install`) завантажується з cgit і сканується у два шари:

1. **AST** (`tree-sitter-bash`): `curl|sh`, `base64`/`xxd | sh`, `source <(curl …)`, `eval "$(curl …)"`, `sudo`/`pkexec`/`doas`, inline-виконання для python/perl/ruby/node/lua, `sh -c` із підозрілим навантаженням, підстановка команд на верхньому рівні, обходи через конкатенацію рядків тощо.
2. **Рядкові евристики:** `chmod 777`, «голі» IPv4, сторонні менеджери пакунків (`npm install …`), paste-сайти, `.onion`, перевірки на антипісочницю, відомі IOC кампаній (Atomic Arch, openconnect-sso, `compromised.txt` 2018 року, …).

Знахідки бувають `Suspicious` або `ConfirmedIoc`. Спрацювання не блокує жорстко: ви отримуєте файл/рядок, посилання на cgit і `Continue anyway? [y/N]`.

### Пісочниця

Збірки **AUR та ABS** виконують `pkgver()` / `prepare()` / `build()` / `check()` / `package()` усередині [`bwrap`](https://github.com/containers/bubblewrap). Це обов'язково; відмовитися можна лише явним `--no-sandbox`.

- порожній tmpfs поверх `/home` і `/run` (немає справжнього `$HOME`, немає сесійної D-Bus / ssh-agent / gpg-agent);
- `--clearenv` плюс невеликий білий список (`PATH`, `TERM`, `LANG`, каталоги призначення збірки, …);
- копія в'язки ключів GnuPG лише з публічними ключами;
- PKGBUILD і `*.install` повторно монтуються лише для читання;
- запис лише в каталог збірки (+ налаштовані `PKGDEST` / `SRCDEST` / …);
- мережа за замовчуванням спільна; інші простори імен вимкнено;
- `fakeroot` у `package()` працює у вкладеному `bwrap` з uid 0.

| Прапор | Ефект |
|--------|-------|
| `--no-sandbox` | Вимкнути bwrap; звичайний `makepkg -si` |
| `--unshare-net-build` | Мережа лише для завантаження/`prepare()`; `build()`/`check()`/`package()` - без мережі |

Якщо `bubblewrap` не встановлено, збірка відхиляється.

**Третій бар'єр перед встановленням:** готовий `*.pkg.tar.*` перевіряється через `bsdtar` на setuid/setgid, скрипти `.INSTALL` і чутливі шляхи (хуки alpm, юніти systemd, sudoers, udev, …). `.INSTALL` повторно сканується сканером PKGBUILD. З `-a` потрібно підтвердити; з `--noconfirm` - попередження і продовження.

Встановлення залежностей (`pacman -S`) і фінальний `pacman -U` виконуються **поза** пісочницею - вони не запускають код PKGBUILD.

### Інше посилення

- Текст новин Arch очищується (не можна впровадити ANSI/bidi в термінал чи `news.state`).
- Файли конфігу, маски, env і world відкриваються з `O_NOFOLLOW`, тож символьне посилання не можна підсунути через гонку.
- Rust-залежності перевіряються через [`cargo deny`](https://github.com/EmbarkStudios/cargo-deny) (`make deny`).

<details>
<summary><h3>Перевірка сканера (після встановлення)</h3></summary>

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
| `-s` | Пошук пакунків |
| `-S`, `--searchdesc` | Пошук за описом |
| `-sv` | Докладна інформація про пакунок |
| `-u` | Повне оновлення системи (world не читає) |
| `--devel` | З `-u`: заодно перезібрати `-git`/`-hg`/`-svn`/`-bzr`, чий upstream пішов уперед |
| `--check-devel` | Показати devel-пакунки зі змінами в upstream (без перезбірки) |
| `-C` | Видалити пакунки (unmerge) |
| `-c` | Видалити сиріт (depclean) |
| `-p` | Pretend (пробний прогін) |
| `-a` | Запитати перед дією |
| `--sudoloop` | Тримати мітку часу sudo «теплою» весь запуск |
| `-1` | Встановити як залежність (oneshot) |
| `-v` | Докладний вивід |
| `-n` | Не перевстановлювати, якщо вже є (noreplace) |
| `--aur` | Примусово AUR (залежності лише з AUR завжди збираються рекурсивно) |
| `--repos` | Лише офіційні репозиторії |
| `--abs` | Збірка з ABS (`pkgctl`) у пісочниці bwrap |
| `--no-sandbox` | Вимкнути bwrap для AUR/ABS |
| `--unshare-net-build` | Без мережі під час build()/check()/package() |
| `--skippgp` | Пропустити перевірки PGP (ABS) |
| `--autopgp` | Автоімпорт відсутніх PGP-ключів (ABS) |
| `--edit` | Редагувати PKGBUILD перед збіркою; після збереження перегенерує `.SRCINFO` |
| `--pkgbuild-view` | Показати/порівняти PKGBUILD і підтвердити перед збіркою (у разі відмови пропонує `$EDITOR`) |
| `--install-pkgbuild <PATH>` | Зібрати й встановити локальний каталог із PKGBUILD (сканер + bwrap) |
| `--scan` | Лише аудит PKGBUILD/.install, без збірки/встановлення (AUR або `--install-pkgbuild`) |
| `--skip-srcinfo-regen` | З `--edit`: не перегенеровувати `.SRCINFO` |
| `--sync` | Синхронізувати бази пакунків |
| `--refresh` | Примусовий `-Syy` (з `--sync` або `-u`) |
| `--info` | Інформація про систему |
| `--news [N\|all]` | Новини Arch Linux |
| `--check-news [N\|all]` | Псевдонім `--news` |
| `--select` | Додати у world без встановлення |
| `--deselect` | Прибрати зі world без видалення |
| `--prune` | Видалити пакунки, яких немає у world |
| `--regen` | Перестворити кеш метаданих |
| `--regen-world` | Заново визначити префікси репозиторіїв у world |
| `--regen-sets <SET>` | Заново визначити префікси для одного власного набору |
| `--regen-sort` | З `--regen-sets`: заодно відсортувати файл |
| `--regen-world-from-explicit` | Міграція: заповнити world із `pacman -Qeq` |
| `--err-install` | З `@world`: заодно повторювати записи `Err/` |
| `--resume` | Продовжити перервану транзакцію |
| `--skipfirst` | З `--resume`: пропустити перший пакунок |
| `--undo` | Скасувати останнє встановлення/видалення |
| `--clean-source-cache` | Очистити постійний кеш джерел VCS |
| `--batchinstall <FILE>` | Масове встановлення з файлу зі списком пакунків |
| `--list-sets` | Список доступних `@наборів` |
| `--exclude <ATOM>` | Виключити пакунок із цього запуску (повторюваний/через кому) |
| `--keep-going` | Не зупиняти пакетну операцію на першій помилці; підсумок наприкінці |
| `--revdep-rebuild` | Знайти й полагодити бінарники, що лінкуються з відсутньою бібліотекою |
| `-t`, `--tree` | Показати план як дерево залежностей |
| `-D`, `--deep[=N]` | З `-t`: вкладеність на N рівнів, без значення - на всі |
| `--ignore-default-opts` | Ігнорувати `EMERGE_DEFAULT_OPTS` з make.conf на цей запуск |
| `--debug` | Показувати вивід makepkg/компілятора наживо (також `AE_DEBUG=1`) |
| `--log <PATH>` | Записати лог сесії (команда, рядки emerge.log, повний вивід збірки) для баг-репортів |
| `-h`, `--help` | Довідка |
| `-V`, `--version` | Версія |

Сторінка посібника `emerge(1)` генерується з CLI (`emerge --gen-manpage`), тому не може розійтися з `--help`.
</details>

## Автор / Ліцензія
Copyright (C) 2026 **Undercat037**
Ліцензія GPL-3.0
