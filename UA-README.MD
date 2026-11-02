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

**Розділи:** [Встановлення](#встановлення) · [Приклади використання](#приклади-використання) · [Конфігурація](#конфігурація) · [Робота з пакунками](#робота-з-пакунками) · [Репозиторії](#репозиторії) · [Безпека](#безпека) · Прапори (В кінці)

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
<summary><h3>Збірка з початкового коду (нестабільно, beta версії)</h3></summary>

```bash
git clone https://github.com/Undercat037/aura-emerge.git
cd aura-emerge
make install            # або вручну:
```

```bash
cargo build --release
sudo install -Dm755 -o root -g root target/release/aura-emerge /usr/bin/emerge
sudo ln -sf /usr/bin/emerge /usr/bin/portageq
sudo ln -sf /usr/bin/emerge /usr/bin/eselect
emerge --gen-completions bash | sudo tee /usr/share/bash-completion/completions/emerge
emerge --gen-completions zsh  | sudo tee /usr/share/zsh/site-functions/_emerge
emerge --gen-completions fish | sudo tee /usr/share/fish/vendor_completions.d/emerge.fish
emerge --gen-manpage | sudo tee /usr/share/man/man1/emerge.1 >/dev/null
sudo install -Dm644 misc/eselect.bash-completion /usr/share/bash-completion/completions/eselect
sudo install -Dm644 misc/_eselect /usr/share/zsh/site-functions/_eselect
sudo install -Dm644 misc/eselect.fish /usr/share/fish/vendor_completions.d/eselect.fish
```
</details>

**Залежності:** `git`, `sudo`, `bubblewrap` (пісочниця збірки AUR/ABS), `pacman` (libalpm, makepkg), `gnupg` (імпорт ключів для внутрішньої реалізації `pacman-key` та `--autopgp` для ABS). 

**Опційно:** `devtools` (підтримка `--abs`).

## Приклади використання

```bash
# Встановлення
emerge neovim                       # офіційні репозиторії, у разі невдачі — AUR
emerge neovim --nodeps              # встановлення пакета, не встановлюючи залежності
emerge neovim-git --aur             # лише AUR (залежності лише з AUR збираються рекурсивно)
emerge neovim --repos               # лише офіційні репозиторії
emerge neovim --abs                 # збірка з ABS (pkgctl) у пісочниці
emerge aur/neovim-git abs/nano cachyos-extra-v3/neovim   # джерело кожного пакунка — префіксом
emerge -pt neovim                   # пробний прогін, план деревом
emerge -a neovim                    # запитати перед дією
emerge -1 htop                      # як залежність (не потрапляє у world)
emerge -at aur/setrixtui abs/nano @fonts --jobs=7 --jobsa=2 # встановлееня по декілька пакетів одночасно, 7 пакетів за раз з репо, збірки 2 одночасно з AUR/ABS

# Видалення
emerge -C neovim                    # видалити пакунок (--unmerge)
emerge -c                           # видалення сиріт (--depclean)
emerge -cp --show-protected         # сироти, а також ті, що блокуються через world, package.protect, або є у обраному профілі
emerge --prune                      # видалити все, чого немає у world

# Оновлення
emerge --sync                       # оновити бази пакунків
emerge -u                           # повне оновлення (репозиторії + AUR)
emerge -u --devel                   # ...і перезібрати -git/-hg/-svn/-bzr з новим upstream
emerge -u --exclude linux --keep-going # Виключення пакету `linux` з оновлення

# World і набори
emerge @world                       # встановити все зі world, те, ще відсутнє
emerge @desktop neovim              # власний набір + звичайний пакунок
emerge --select neovim              # додати у world без встановлення
emerge --deselect neovim            # прибрати зі world без видалення

# Пошук та інформація
emerge -s neovim                    # пошук: репозиторії => ABS => AUR
emerge -S neovim                    # пошук за описом
emerge -sv neovim                   # докладно
emerge -s abs/nano                  # лише ABS (aur/ і <repo>/ теж працюють)
emerge --info                       # інформація про систему
emerge --news                       # новини Arch Linux

# Безпека
emerge neovim-git --aur --pkgbuild-view   # показати/порівняти PKGBUILD перед збіркою
emerge neovim-git --aur --edit            # спершу відредагувати PKGBUILD
emerge --scan neovim-git --aur            # лише аудит, без збірки
emerge --usepkgbuild ~/src/my-pkg         # локальний PKGBUILD через сканер і пісочницю
emerge --usepkg ./foo-1.0-1-x86_64.pkg.tar.zst  # готовий архів (pin + аудит)

# Без встановлення
emerge -B neovim --aur              # лише зібрати архіви
emerge -f neovim --aur --outdistfiles ~/ae-out   # лише завантажити все дерево в обрану директорію
emerge --usepkg ~/ae-out            # поставити всі архіви з каталогу однією транзакцією

# Відновлення
emerge --resume                     # продовжити перервану транзакцію
emerge --undo                       # скасувати останнє встановлення/видалення/depclean
emerge --revdep-rebuild             # полагодити бінарники зі зламаними посиланнями на .so
emerge --aelist                     # файли, залишені AE Force
```

Повна таблиця прапорів — наприкінці сторінки.

## Конфігурація

Усе налаштовується в `/etc/portage/`. Конфіг лише системний і належить root.

| Шлях | Призначення |
|------|-------------|
| `make.conf` | Головний конфіг: прапори за замовчуванням, кольори, прапори збірки, `FEATURES` |
| `package.mask`, `package.unmask` | Що ніколи не встановлювати / винятки |
| `package.protect`, `package.freeze` | Що не можна видаляти / що не оновлювати |
| `package.env`, `env/` | Прапори збірки для окремих пакунків |
| `make.profile` | Профіль (символічне посилання) |
| `world`, `sets/` | Явно встановлені пакунки та власні набори |
| `repos.conf`, `ae-repos.conf` | Список репозиторіїв (`FEATURES="repos"`) і секції `ae-*` для `pacman.conf` |
| `hooks/` | Власні alpm-хуки |
| `resume.state`, `lastaction.state` | Стан для `--resume` і `--undo` |

Інше: `/var/lib/aura-emerge/repos/` — локальні репозиторії, `/var/log/emerge.log` — журнал операцій, `~/.cache/aura-emerge/` — кеші (PKGBUILD-view, devel, новини, збірки), /var/db/repos/portage/profiles/ — база профілів.

### make.conf

```bash
# Прапори за замовчуванням. Підставляються перед набраним (командний рядок головніший).
# Вимкнути для одного запуску: --ignore-default-opts.
EMERGE_DEFAULT_OPTS="--jobs=8 --tree --deep"

# Кольори рядків статусу (ролі: ok, warn, error, info, special).
COLORS="ok=#FFB3AE warn=#E7BDB9 error=#FDB1AC info=#E7BDB9 special=#cba6f7"

# Прапори збірки (див. нижче)
CFLAGS="-march=native -O2 -pipe"
CXXFLAGS="$CFLAGS"
RUSTFLAGS="-C target-cpu=native"
MAKEFLAGS="-j$(nproc)"
NINJAFLAGS="-j$(nproc)"
OPTIONS=(strip docs !debug lto)
BUILDENV=(!distcc color check !sign)

# Перемикачі лише для aura-emerge
FEATURES="localrepo"
```

Синтаксис як у bash: `KEY="..."`, `KEY='...'` (буквально, без розкриття) або `KEY=(a b c)`. У подвійних лапках і без лапок працюють `$(nproc)` та `$CFLAGS`. Файл читається з `O_NOFOLLOW`: символьне посилання відхиляється.

| Ключ | Призначення |
|------|-------------|
| `EMERGE_DEFAULT_OPTS` | Прапори, що підставляються в кожен запуск |
| `COLORS` | Палітра статусного виводу |
| `FEATURES` | Перемикачі aura-emerge |
| `EMERGE_FREEZE` | Те саме, що `package.freeze`, одним рядком |
| `CFLAGS` `CXXFLAGS` `CPPFLAGS` `LDFLAGS` `RUSTFLAGS` `MAKEFLAGS` `NINJAFLAGS` `BUILDENV` `OPTIONS` | Прапори збірки AUR/ABS |

**`EMERGE_DEFAULT_OPTS`** приймає лише прапори-модифікатори (`--ask`, `--verbose`, `--devel`, `--tree`, `--deep`, `--jobs=N`, `--jobsa=N`, `--searchdesc`, `--exclude=<atom>` тощо), тож зробити з кожного запуску `--unmerge` не вдасться. Суперечливі пари відхиляються, якщо задані обидві: `--aur`/`--abs`, `--aur`/`--repos`, `--no-sandbox`/`--unshare-net-build`, `--skippgp`/`--autopgp`, `--select`/`--deselect`. Значення з конфігу, що суперечить командному рядку, відкидається. `--abs` і `--repos` **не** суперечать одне одному.

**`COLORS`** — токени `роль=колір`; колір — `#RRGGBB` або ім'я (`red`, `bright-cyan`, …). Останнє згадування ролі перемагає, невідомі ролі й кольори пропускаються з попередженням.

**`FEATURES`** — лише для aura-emerge, у makepkg ніколи не перекладається (ccache, check, strip задавайте в `BUILDENV` / `OPTIONS`). `-name` вимикає токен, перемагає остання згадка. Токени:

| Токен | Що вмикає |
|-------|-----------|
| `localrepo` | локальні репозиторії `ae-aur` / `ae-abs` / `ae-local` ([докладніше](#локальні-репозиторії)) |
| `repos` | Portage-style `repos.conf` ([докладніше](#reposconf)) |

Невідомі токени читаються й показуються в `--info`, але нічого не роблять. `emerge --info` друкує чинні `FEATURES` і значення, що перекривають makepkg. [**=> Приклад make.conf**](misc/make.conf)

#### Gentoo => aura-emerge

| Gentoo (`make.conf`) | aura-emerge (`make.conf`) |
|----------------------|---------------------------|
| `CFLAGS`, `CXXFLAGS`, `RUSTFLAGS` | так само |
| `MAKEOPTS="-j$(nproc)"` | `MAKEFLAGS="-j$(nproc)"` |
| `USE="lto"` | `OPTIONS=(... lto)` |
| `FEATURES="ccache"` | `BUILDENV=(... ccache)` |
| `FEATURES` (функції Portage) | `FEATURES` лише для AE: `localrepo`, `repos` |

USE-прапорів в Arch немає, тож їхню роль частково виконує `OPTIONS`. Довідка: [прапори оптимізації GCC](https://gcc.gnu.org/onlinedocs/gcc/Optimize-Options.html), [параметри кодогенерації rustc](https://doc.rust-lang.org/rustc/codegen-options/index.html).

### Прапори збірки і makepkg.conf

Прапори збірки задаються лише в `make.conf` (і точково в [`package.env`](#packageenv)). Правити `/etc/makepkg.conf` заради aura-emerge не треба.

Перед кожною збіркою AUR/ABS emerge сам генерує тимчасовий `makepkg.conf`: спершу він підключає системний, потім перекриває його ключами з `make.conf`, і цей файл передається в makepkg через `--config`. Тому:

- системний `/etc/makepkg.conf` — лише база для ключів, яких у `make.conf` немає (`PKGEXT`, стиснення, `DLAGENTS` та ін.); за збігу ключів завжди перемагає `make.conf`;
- якщо в `make.conf` немає жодного ключа збірки, makepkg лишається без змін;
- у пісочниці користувацький `~/.config/pacman/makepkg.conf` не діє (`$HOME` порожній); без пісочниці (`--no-sandbox`) makepkg підключає його *після* згенерованого, і збіг ключів перемагає make.conf — emerge про це попереджає.

Щодо безпеки: значення екрануються під час генерації (`"`, `` ` ``, `\`; значення в одинарних лапках не розкривається взагалі), у `BUILDENV` / `OPTIONS` дозволені лише токени з літер, цифр і `!_-+.` (інакше попередження, і ключ лишається як у системному файлі). Згенерований файл лежить у приватному (`0700`) службовому каталозі поза деревом збірки, щоб `prepare()` / `build()` не могли його переписати, і видаляється після збірки.

### package.env

Прапори збірки для окремих пакунків. Запис — атом і один чи кілька env-файлів із `/etc/portage/env/`. `/etc/portage/package.env` — файл або каталог.

```
# /etc/portage/package.env/neovim
abs/neovim  lto.conf            # префікс репозиторію і '*' працюють як у package.mask
aur/*-git   no-lto  fast        # кілька env застосовуються в указаному порядку
```

```bash
# /etc/portage/env/lto.conf  (синтаксис make.conf; враховуються лише ключі збірки)
CFLAGS="-march=znver3 -O2 -pipe"
MAKEFLAGS="-j16"
OPTIONS="strip docs !debug lto"
```

- **Накладання:** `make.conf` => відповідні записи `package.env` => кожен env-файл за порядком. Перемагає шар, що задав ключ останнім; ключ замінюється повністю, а не зливається. Збірка повідомляє, який шар переміг.
- **Збіг:** за pkgbase або будь-яким із pkgname.
- **Область:** лише збірки AUR/ABS. `EMERGE_DEFAULT_OPTS` в env-файлі ігнорується з попередженням.
- **Помилки:** символьне посилання, некоректний запис або відсутній env-файл пропускаються з попередженням (файл і рядок), решта застосовується.

### package.mask і package.unmask

`package.mask` — пакунки, які ця машина не повинна встановлювати ніколи. Перевірка спрацьовує для будь-якого шляху: запит за іменем, транзитивна залежність з AUR, провізіонінг world, `--select`/`-R`/`-u`. Замаскований пакунок жорстко зупиняє запит, без питань і без прапора для обходу; обійти можна лише записом у `package.unmask`.

`/etc/portage/package.mask` — файл або каталог (читається кожен звичайний файл за порядком імен; файли з крапкою на початку пропускаються; символьні посилання відхиляються). Один запис на рядок, дозволені `*`, префікс репозиторію й коментар `# …` наприкінці.

```
ayugram-desktop-bin            # конкретний пакунок, з будь-якого джерела
aur/*-bin                      # ніколи не ставити -bin з AUR
*-git                          # на цій машині без devel-пакунків
extra/nano                     # конкретний пакунок з конкретного репозиторію
```

Префікс репозиторію враховується всюди, де джерело відоме (`extra/nano` спрацьовує лише для `extra`, `aur/foo-git` — лише для AUR). Версійні атоми Portage (`=cat/foo-1.2`, `>=cat/foo`) не підтримуються: зіставлення йде за назвою, такі записи виводяться попередженням і пропускаються. Маска діє постійно, на відміну від `--exclude`, який діє на один запуск.

`package.unmask` (такий самий синтаксис) перелічує винятки: пакунок, що підпадає під запис, проходить, навіть якщо його блокує `package.mask` або маска активного профілю. Root-helper це теж враховує.

```
aur/some-git                   # дозволений, хоча *-git замасковані вище
```

### package.protect і package.freeze

**`package.protect`** — пакунки, які не можна видаляти (синтаксис як у `package.mask`). Діє на `-C`, `-c`, `--prune` і видалення через конфлікти. `--deselect` усе ще може прибрати ім'я зі world, але пакунок лишиться встановленим, і depclean його не чіпатиме. Пакунки `@system` з профілю захищені так само.

**`package.freeze`** (і необов'язковий `EMERGE_FREEZE` у `make.conf`) тримає пакунки на встановленій версії під час `-u`, включно з перезбірками AUR/ABS. Суворіше за `IgnorePkg` у pacman: діє і на вибірковий `emerge -u pkg`.

### Профілі

`/etc/portage/make.profile` — символьне посилання на каталог профілю (розкладка Portage). `--list-profiles` показує дерево з `/var/db/repos/portage/profiles`; `parent` виду `repo:path` шукається в `/var/db/repos/<repo>/profiles/`. Профіль може містити:

- `parent` — ланцюжки розгортаються вглиб;
- `package.profile` — `*` означає стартовий набір `@system` (стара назва `packages` теж читається);
- `package.mask` — `-atom` або `!atom` знімає маску, успадковану від батька;
- `package.protect`, `make.defaults`.

`emerge @system` встановлює системний набір профілю; усі записи `package.profile` захищені від depclean так само, як world.

```bash
emerge --list-profiles
sudo emerge --set-profile 12
# або через eselect лінк: eselect profile set 12
```

`/usr/bin/eselect` => `emerge` дає мінімальний `eselect` (лише `eselect profile list|set|show`). Для `--list-profiles` і `--set-profile` він не потрібен.

### world і набори

**`/etc/portage/world`** — список усіх явно встановлених пакунків. Записи додаються при встановленні, прибираються при видаленні й редагуються вручну (один на рядок).

```
core/nano
extra/firefox
aur/ayugram-desktop-bin
abs/nano
local/my-pkg
chaotic-aur/visual-studio-code-bin
```

**Префікси:** назва офіційного репозиторію, `aur/`, `abs/`, `local/` (`--usepkgbuild` і `--usepkg`), `Err/` (джерело невідоме).

> **Про `Err/`:** встановлений пакунок, чиє походження визначити не вдалося, або запис `aur/` / `abs/`, що перестав розв'язуватися. `--regen-world` і `--regen-sets` зберігають відомий префікс, а не відкочуються до `Err/`. Під час `emerge @world` такі записи лише перелічуються; встановити їх можна з `--err-install`.

Якщо в пакетному встановленні одного імені не знайдено, решта все одно встановлюється; відсутні виводяться наприкінці, код виходу ненульовий.

**`@world` має два значення:**

| Команда | Дія |
|---------|-----|
| `emerge @world` | *Підготовка:* ставить те, що є у world, але ще не встановлено. Тримайте world у dotfiles, клонуйте на нову машину, запускайте |
| `emerge -u` / `emerge -u @world` | *Оновлення:* оновлює все встановлене (репозиторії + AUR); world не читає |
| `emerge --sync` | Лише оновлення баз (`-Sy` / `-Syy`) |

**Власні набори.** `/etc/portage/sets/<name>` (або `<name>.set`) викликається як `@<name>`. Один пакунок на рядок, коментарі `#` дозволені; рядок `@other` вкладає інший набір (рекурсивно, цикли безпечні). Встановлення `@name` записує `@name` у world, тож члени набору захищені від depclean, доки їх не приберуть із файлу набору. Якщо існують і `game-kit`, і `game-kit.set`, `@game-kit` відхиляється як неоднозначний.

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
emerge -s @game-kit                   # звідки береться кожен пакунок: repo / aur / not found
emerge --batchinstall pkgs.txt        # той самий формат, але файл будь-де і без реєстрації
```

`@world` і `@preserved-rebuild` — дії, тож `-s @world` і `-s @preserved-rebuild` дають помилку.

## Робота з пакунками

### Джерела й префікси

Без префікса emerge шукає в офіційних репозиторіях, а за невдачі — в AUR. Префікс обирає джерело: `aur/<name>`, `abs/<name>`, `<repo>/<name>`. Префікси працюють у командах, у `world`, у наборах і в `-s`. `--aur`, `--repos`, `--abs` задають джерело для всього запуску; залежності, що є лише в AUR, збираються рекурсивно.

### Паралельні jobs і прогрес

`--jobs=N` ставить до N пакунків з офіційних репозиторіїв одночасно, `--jobsa=N` — до N збірок AUR/ABS (за замовчуванням 1 і 1). `--load-average=X` не запускає нові збірки, доки навантаження за хвилину вище за X (але ніколи, якщо більше нічого не працює). `--nospinner` прибирає живий рядок Jobs. Значення за замовчуванням зручно тримати в `EMERGE_DEFAULT_OPTS`.

```
>>> Emerging (3 of 12, 16%) noctalia-5.1.0-1.1
>>> Jobs: 26 of 26 complete  2m53s                         Load avg: 2.49, 3.11, 2.17
```

Під час тихої збірки кожні 30 с друкується heartbeat (pid, RSS: `still building … — process alive`); якщо процес убито сигналом (OOM/SIGKILL), це повідомляється явно. Якщо helper відхилив один пакунок із пачки (наприклад, він замаскований), пачка повторюється по одному, щоб решта все одно встановилася. `--debug` (або `AE_DEBUG=1`) показує вивід компілятора наживо.

### План деревом: `-t` і `--deep`

Показує план (з `-p` або перед справжнім встановленням) деревом залежностей. За замовчуванням — один рівень; `--deep` іде до дна, `--deep=N` — на N рівнів.

```
[ebuild  N ] noctalia-5.1.0-1.1
[ebuild  N ] `-- libqalculate-5.12.0-1.1
[ebuild  N ]   `-- sdbus-cpp-2.3.1-1.1
```

Залежність, яку не названо в чиємусь «Depends On» (розв'язано через `provides` або глибша за `--deep`), усе одно показується, але без батька. Для чистого встановлення через `--aur` транзитивні залежності лише з AUR на кроці плану поки не розв'язуються, тож дерево там пласке.

### `--exclude` і `--keep-going`

`--exclude <atom>` (повторюваний або через кому) виключає пакунок із запуску: встановлення, `-u`, `@world`, `--depclean`/`--prune` (захищає від видалення) і `--revdep-rebuild`.

`--keep-going` не зупиняє пакетну операцію (`@world`, `-u`, `--batchinstall`, кілька пакунків) на першій помилці: збої запам'ятовуються, наприкінці друкується підсумок і повертається ненульовий код. Наступний `--resume` повторить лише те, що впало.

```bash
emerge -u --exclude linux --exclude aur/ayugram-desktop-bin
emerge -u --keep-going
emerge --resume
```

### Конфлікти (блокери)

Встановлені пакунки, що блокують вхідний, показуються до початку встановлення, як блокери Portage:

```
[ebuild  N    ] pipewire-jack-1:1.4.2-1
[conflict     ] jack2-1.9.22-1 ("jack2" is blocking pipewire-jack-1:1.4.2-1)
```

Перевірка охоплює всі джерела запуску: план із репозиторіїв (libalpm), пакунки AUR і їхні AUR-залежності, пакунки ABS (за `.SRCINFO`: `provides`/`conflicts`) і голий `@world`. Джерела перевіряються й **одне проти одного**: якщо два пакунки одного запуску блокують один одного (скажімо, з репозиторію і з AUR), це показується як `[blocks]`, і нічого не встановлюється. Для встановлених блокерів потрібен один `Yes` (питається завжди); спершу перелічуються пакунки, яким вони потрібні, а записи `package.protect` не видаляються ніколи. Те, чого попередня перевірка не побачила (наприклад, немає мережі для `.SRCINFO`), усе одно ловиться під час встановлення.

### Файли без власника: AE Force

Якщо встановлення перезаписало б файл, яким не володіє жоден пакунок, emerge питає `AE Force: Create .aenew for conflict files?`. «Так» залишає ваш файл на місці, а версію з пакунка зберігає як `<file>.aenew`.

| Прапор | Що відбувається |
|--------|-----------------|
| `--aeforce` | без питання; ваш файл переноситься до `<file>.aeforce`, новий займає його місце |
| `--noaefile` | без питання; файлів `.ae*` не лишається: старий збережено, новий відкинуто (з `--aeforce`: старий відкинуто, новий збережено) |

`<file>.aepark` лишається після перерваного встановлення; якщо ім'я зайняте, додається суфікс `.N`. `emerge --aelist` перелічує всі такі файли, `emerge --aeclean [new|old|park|all]` видаляє їх після питання (без аргументу = `all`).

### Хуки

libalpm виконує кожен `*.hook` під час встановлення, видалення, `-U` та `-Su`, як і pacman (depmod, DKMS, mkinitcpio/dracut, кеші fontconfig та іконок, `grub-mkconfig`, …). За збігу імен перемагає ранніший каталог:

```
/usr/share/libalpm/hooks/    хуки з пакунків
/etc/pacman.d/hooks/         хуки адміністратора
/etc/portage/hooks/          власні хуки aura-emerge (формат alpm-hooks(5))
```

Усі три реєструються завжди, навіть без `HookDir` у `pacman.conf`. Хід виконання: `>>> Running pre-transaction hooks...` / `>>> (2 of 4) Install DKMS modules`.

### Оновлення

`emerge -u` оновлює репозиторії та AUR (world не читає). Дві речі, яких бракує звичайному оновленню:

**Devel-пакунки (`-git`, `-hg`, `-svn`, `-bzr`).** Версія в AUR змінюється, лише коли супровідник перезаливає пакунок, тож `-u` часто не бачить змін в upstream. `-u --devel` (оновлення) і `--check-devel` (лише звіт) закривають цю прогалину: для кожного встановленого devel-пакунка беруть URL `git+` з PKGBUILD, роблять `git ls-remote` (клон не потрібен) і порівнюють з хешем минулого разу (`~/.cache/aura-emerge/devel.state`). Поки що лише git; `-hg`/`-svn`/`-bzr` виводяться як «couldn't determine», а не пропускаються мовчки.

**Зламані посилання на бібліотеки.** В Arch немає обліку preserved-libs, тож є два інструменти:

- `@preserved-rebuild` збирає *оголошені* залежності всіх встановлених пакунків (`pacman -Qi`), перевіряє їх через `pacman -T` і пропонує перевстановити відсутні з `--asdeps` (щоб не потрапили у world через `--regen-world-from-explicit`);
- `--revdep-rebuild` читає ELF-файли (`DT_NEEDED`, `DT_RPATH`/`DT_RUNPATH`) і знаходить бінарники, що лінкуються з неіснуючим `.so` (класична поломка після оновлення `openssl`/`icu`/`boost`). Пакунки AUR/локальні перезбираються через пісочницю; пакунки офіційних репозиторіїв перезібрати не можна, тож відсутня бібліотека шукається через `pacman -F`, а пакунок, що її надає, пропонується до встановлення. `--exclude` і [маска](#packagemask-і-packageunmask) діють.

```bash
emerge -u --devel
emerge --check-devel
emerge @preserved-rebuild -p     # лише список
emerge --revdep-rebuild -p       # лише скан
emerge --revdep-rebuild          # скан, потім питання перед виправленням
```

### Видалення

`-C` видаляє вказані пакунки, `-c` — сиріт, `--prune` — усе, чого немає у world. `-C` друкує, що `selected`, `protected` і `omitted`; потім `-C` і `-c` чекають 5 секунд (`Unmerging in: 5 4 3 2 1`, Ctrl-C перериває), якщо тільки `-a` уже не запитав або не задано `-p`. Після `-C` імена також прибираються зі world.

`-c` ніколи не видаляє запис world, члена набору, записаного у world, запис `package.protect` і пакунок `@system` з профілю. Він повідомляє, скільки пропущено; `--show-protected` перелічує їх. З аргументами `-c` працює як залежностеобізнаний unmerge: видаляються лише пакунки, потрібні нікому (`-cv` показує зворотні залежності, що блокують видалення).

```bash
emerge -cp --show-protected
emerge -c
emerge --unmerge --ask linux-zen-headers
```

### Пошук та інформація

`-s` шукає в офіційних репозиторіях, потім в ABS, потім в AUR. `-s aur/<name>` — лише AUR, `-s abs/<name>` — ABS, `-s <repo>/<name>` — спершу цей репозиторій; `--repos` обмежує пошук офіційними репозиторіями; `--search-all` показує те саме ім'я з кожного репозиторію. Замасковані пакунки позначаються `[ Masked ]`. `-sv` додає встановлену версію, а для AUR ще й мейнтейнера, голоси, популярність, домашню сторінку, дату оновлення та ліцензію.

`emerge --info` друкує версію (і версію libalpm), ядро, CPU/пам'ять/swap, чинні `--jobs`/`--jobsa`, кожен репозиторій у порядку `pacman.conf` із дзеркалом, значення з `makepkg.conf` і `make.conf` (і які перекриття перемогли), `FEATURES`, профіль, розміри mask/world та підсумки журналу merge.

`emerge --pkginfo nano` — картка встановленого пакунка (як короткий `pacman -Qi`); з `-v` додатково перелічує файли.

### Збірка без встановлення

| Прапор | Що робить |
|--------|-----------|
| `-B`, `--buildpkgonly` | Лише архіви, без встановлення: офіційні => у `CacheDir`; AUR/ABS/локальні => збираються |
| `-f`, `--fetchonly` | Лише завантаження всього дерева залежностей: офіційні => архіви в `CacheDir`; AUR/ABS => clone + джерела, перевірені в пісочниці (`makepkg --verifysource`, у `SRCDEST`), для пакунка та його AUR/ABS-залежностей; офіційні залежності — архівами. Без збірки й встановлення |
| `--outdistfiles <DIR>` | З `-B`/`-f`: скопіювати архіви (разом із залежностями) у DIR; checkout'и AUR/ABS лінкуються туди ж |
| `-O`, `--nodeps` | Без розв'язання залежностей: офіційні цілі (`-Sdd`); з `--usepkg` — один архів (`-Udd`). AUR/ABS: не реалізовано, виводиться попередження |
| `-o`, `--onlydeps` | Лише залежності офіційних цілей (не самі цілі), неявно `-1`. AUR/ABS: не реалізовано |

`-B` і `-f` не користуються локальними бінарниками з `ae-*`. Отримане можна розгорнути пізніше: `emerge --usepkg <DIR>` ставить усі архіви з каталогу однією транзакцією, `emerge --usepkg -O <file>` — один архів без залежностей.

### `--resume` і `--undo`

`--resume` продовжує перервану транзакцію (`--skipfirst` пропускає перший пакунок). `--undo` відкочує останню успішну операцію (один крок історії):

- **після встановлення** — видаляє лише пакунки, що були в цьому запуску новими, і повертає встановлені пакунки, видалені як блокери;
- **після видалення** — перевстановлює із записаного джерела (`Err/` і `abs/` лише перелічуються);
- **після depclean** — перевстановлює видалених сиріт **як залежності** (у world не додаються): пакунки з репозиторіїв — з репозиторіїв, локальні збірки — через AUR, якщо він їх ще містить. Версії записуються: якщо в репозиторіях тепер інша, вам про це скажуть; пакунок, якого ніде немає, перелічується з рядком `pacman -U` для файлу з кешу. Якщо частина не встала, у збереженому стані лишаються тільки вони, і `--undo` можна запустити знову;
- **після `-u`** — не підтримується (немає знімка версій).

### Журнал і новини

`/var/log/emerge.log` — лише дозапис, один рядок на подію; збій запису ніколи не ламає merge. Пакунки AUR/ABS збираються по одному, тож у кожного свій таймер; `pacman -S`/`-Syu` — одна транзакція, тому її рядок містить усі атоми й загальний час. `--info` показує підсумки.

```
2026-09-21 08:51:44  MERGE    aur    noctalia-5.1.0-1.1        (12s)
2026-09-21 09:03:02  MERGE    repo   nano-8.0-1  vim-9.1-1     (4s)
2026-09-21 09:10:05  UNMERGE  -      old-package-2.0-1
```

`--log <PATH>` додатково пише лог сесії (команда, рядки emerge.log, повний вивід збірки) — для баг-репортів; лише дозапис.

`emerge --news` отримує [RSS-стрічку новин Arch Linux](https://archlinux.org/feeds/news/); стан прочитаного — у `~/.cache/aura-emerge/news.state` (root не потрібен).

```bash
emerge --news        # список, непрочитані позначені [N]
emerge --news 3      # прочитати запис 3
emerge --news all    # позначити все прочитаним
```

## Репозиторії

### Локальні репозиторії

З `FEATURES="localrepo"` кожен пакунок, який aura-emerge зібрав з AUR, ABS або локального PKGBUILD (або взяв з архіву `--usepkg`), після аудиту й встановлення розкладається в `/var/lib/aura-emerge/repos/<repo>/`, а база репозиторію оновлюється. Каталоги належать root (`0755`). Це аналог aurutils: джерела aur / abs / local розділені на рівні бази libalpm.

| Репозиторій | Що зберігає |
|-------------|-------------|
| `ae-aur` | збірки з AUR |
| `ae-abs` | збірки з ABS (`--abs`) |
| `ae-local` | `--usepkgbuild` і `--usepkg` |

Репозиторії наповнюються в будь-якому разі, але pacman їх **бачить**, лише якщо вони вказані в `/etc/pacman.conf` *після* офіційних (`core` / `extra` / `multilib`). Два способи:

```ini
# 1) підключити готовий файл (misc/ae-repos.conf => /etc/portage/ae-repos.conf)
Include = /etc/portage/ae-repos.conf
```

```ini
# 2) або записати секції самому
[ae-aur]
SigLevel = Never
Server = file:///var/lib/aura-emerge/repos/ae-aur

[ae-abs]
SigLevel = Never
Server = file:///var/lib/aura-emerge/repos/ae-abs

[ae-local]
SigLevel = Never
Server = file:///var/lib/aura-emerge/repos/ae-local
```

Що це дає: `pacman -Qm` більше не показує ці пакунки, працює `pacman -Si ae-aur/<pkg>`, а `pacman -S` / `-Su` можуть перевстановити їх із перевіреної копії. За збігу імен перемагає перший репозиторій, де воно є, тож секції `ae-*` мають стояти останніми — офіційні зберігають пріоритет. `SigLevel = Never` зроблено навмисно: нічого тут не підписано, пакунки перевірені й закріплені sha256 самим emerge, а каталоги належать root, тож підмінити файл нікому.

Що emerge робить далі:

- **Повторне використання:** звичайне встановлення пакунка з AUR, версія якого в `ae-aur` збігається з поточною в AUR, ставиться з локального репозиторію — без clone і збірки. `--edit` та `--pkgbuild-view` примусово перезбирають; `-B` / `-f` локальні бінарники не використовують; явний `ae-aur/<name>` іде звичайним встановленням з репозиторію.
- **Прибирання:** `-C`, `-c` і `--prune` заодно прибирають видалене з `ae-*`. `emerge --localrepo-clean` прибирає всі записи, пакунок яких уже не встановлений (`-p` лише показує, `-a` питає).
- **Порожні бази** створюються хуком `post_install` / `post_upgrade` пакунка, командою `emerge --sync` і першою публікацією, тож `pacman -Sy` не спотикається об перелічений `file://`-сервер без пакунків.
- **Збій публікації** — попередження, а не провал merge. Якщо секцій `[ae-*]` немає, emerge один раз виводить підказку з рядком `Include`.

### repos.conf

Список репозиторіїв у стилі Portage: `/etc/portage/repos.conf` (файл) або `/etc/portage/repos.conf/*.conf`. Вмикається через `FEATURES="repos"`. Тоді `emerge --info` показує порядок за пріоритетом (за спаданням) і пише `/etc/portage/ae-generated-repos.conf`. Підключіть його в pacman **замість** ручного переліку тих самих репозиторіїв: приберіть відповідні блоки `[repo]` з `pacman.conf` і додайте

```ini
Include = /etc/portage/ae-generated-repos.conf
```

| Ключ | Зміст |
|------|--------|
| `priority` | Більше = вищий пріоритет. За рівності — порядок оголошення (лише попередження, числа не переписуються) |
| `sync-uri` | Один або кілька URL (`$repo` / `$arch`) |
| `include` | Шлях до mirrorlist pacman (`Server = …`); сумісно з rate-mirrors. Явні `sync-uri` йдуть першими |
| `verify-signature` | Рядок `SigLevel` як є (`Never`, `Required DatabaseOptional`, …) |
| `location` | Необов'язково (для `file://` — каталог репозиторію) |

Шаблон у поставці — `misc/repos.conf` (секції закоментовані):

```ini
[g14]
priority = 55
sync-uri = https://arch.asus-linux.org
verify-signature = Required DatabaseOptional

[core]
priority = 50
include = /etc/pacman.d/mirrorlist
verify-signature = Required DatabaseOptional

[ae-aur]
priority = 10
sync-uri = file:///var/lib/aura-emerge/repos/ae-aur
verify-signature = Never
```

З `FEATURES="localrepo"` відсутні секції `ae-aur` / `ae-abs` / `ae-local` підставляються з пріоритетами 10 / 9 / 8.

## Безпека

### Привілеї

Фронтенд ніколи не працює від root. Якщо запуск від root (`sudo emerge …`, `doas`, `pkexec`, `su -`), він спершу стартує root-helper, потім назавжди знижує права до користувача, що викликав (береться з `AE_USER`, `SUDO_UID`, `DOAS_USER`, `PKEXEC_UID` або audit login uid; якщо нічого не знайдено — зупинка). Від звичайного користувача helper стартує через `sudo` при першій потребі. Резолвер, збірки AUR/ABS, пісочниця та всі кеші працюють без привілеїв; уся робота з libalpm (install/remove/sync/upgrade/`-U`, перевірка підписів) іде через helper (`--ae-service`), який перевіряє кожен запит і сам застосовує `package.mask` (`helper refused: … masked by …`).

**Умови запуску helper:** бінарник належить root і не group/world-writable (`install -m755 -o root -g root`), uid = 0, stdin/stdout — пайпи (не TTY), батьківський процес живий. Конфлікти чужих файлів на `-Su` / `-U` розв'язуються через AE Force, а не зривають оновлення.

### Сканування PKGBUILD

Перед кожним встановленням з AUR `PKGBUILD` (і будь-який `.install`) завантажується з cgit і сканується у два шари:

1. **AST** (`tree-sitter-bash`): `curl|sh`, `base64`/`xxd | sh`, `source <(curl …)`, `eval "$(curl …)"`, `sudo`/`pkexec`/`doas`, inline-виконання для python/perl/ruby/node/lua, `sh -c` із підозрілим навантаженням, підстановка команд на верхньому рівні, обходи через конкатенацію рядків тощо.
2. **Рядкові евристики:** `chmod 777`, «голі» IPv4, сторонні менеджери пакунків (`npm`/`pnpm`/`bun`/`yarn`/`pip`/`gem install …`, лише віддалені `cargo install`/`go install`), `bash <(curl …)`, `curl … | python3/perl/node/…`, розпакування одразу в шелл (`gunzip`/`zstd -d`/`unzip -p`/`tar -xO | sh`), `/dev/tcp` і `/dev/udp`, heredoc'и, що скармлюються шеллу, paste-сайти, `.onion`, перевірки на антипісочницю, відомі IOC кампаній (Atomic Arch, openconnect-sso, `compromised.txt` 2018 року, …).

Знахідки бувають `Suspicious` або `ConfirmedIoc`. Спрацювання не блокує жорстко: ви отримуєте файл і рядок, посилання на cgit та `Continue anyway? [y/N]`.

**Допоміжні файли.** Коли checkout лежить на диску (збірка або `--usepkgbuild <PATH> --scan`), додатково перевіряються:

- `Makefile` (`Makefile`/`makefile`/`GNUmakefile`) і shell-скрипти, на які посилається PKGBUILD (`*.sh`, `*.bash`, `*.zsh`, `*.fish`) — построково: `curl|sh`, base64/hex у shell, «голі» IP та решта shell-IOC;
- патчі (`*.patch`, `*.diff`, `*.dpatch`, `*.pch` або вміст, схожий на unified/git diff) — за заголовками й доданими рядками: шляхи поза деревом джерел (абсолютні, `..`, `.git`), створення symlink, бінарні hunk'и, надто довгі додані рядки, мережеві виклики в build/script-цілі та ті самі shell-IOC.

`--scan <pkg> --aur` бачить лише те, що віддає cgit (`PKGBUILD` і `.install`).

### Перегляд PKGBUILD і локальні джерела

- **`--pkgbuild-view`** показує PKGBUILD прямо перед збіркою й просить підтвердження: diff з останньою показаною копією (кеш `~/.cache/aura-emerge/pkgbuild-view/`) або весь файл уперше. У разі відмови пропонує відкрити `$EDITOR`, а не просто пропускає пакунок. Працює після `--edit`, тож ви бачите власні правки. Застосовується лише до напряму запитаних пакунків AUR/ABS — не до рекурсивних залежностей і не до пакетних шляхів (`@world`, `-u`, `--undo`).
- **`--usepkgbuild <PATH>`** збирає й встановлює локальний каталог із PKGBUILD тим самим конвеєром, що AUR/ABS (сканер => пісочниця), а не голим `makepkg -si`. У `PATH` має бути `PKGBUILD`; відсутній `.SRCINFO` створюється. Поєднується з `--pkgbuild-view`, `-a`, `-1`, `--skippgp`, `--no-sandbox`, `--unshare-net-build`, `--skip-srcinfo-regen`, але не з пакунками/`@наборами` та `--aur`/`--abs`. У world записується як `local/`, якщо немає `-1`.
- **`--usepkg <FILE…|DIR>`** встановлює готові архіви `.pkg.tar.*` (результат makepkg, пакунок із файлового репозиторію, каталог після `-f --outdistfiles`) без жодного PKGBUILD. Кожен архів фіксується хешем, розпаковується й аудитується, повторно хешується безпосередньо перед привілейованим встановленням і лише тоді передається libalpm (деталі — у розділі «Перевірка готових архівів»). У world — `local/`, якщо немає `-1`. Кілька архівів за запуск — одна транзакція; залежності між ними розв'язуються разом.
- **`--scan`** перевіряє PKGBUILD (+ `.install`), нічого не збираючи й не встановлюючи: для AUR бере файли з cgit, для локального каталогу — `--usepkgbuild <PATH> --scan`. Звіт той самий, що перед збіркою, без питання «Continue anyway?». Ненульовий код при будь-якій знахідці, тож годиться для скриптів. Для `--abs` поки недоступний.

```bash
emerge neovim-git --aur --pkgbuild-view
emerge --usepkgbuild ~/src/my-pkg --pkgbuild-view --skippgp
emerge neovim-git --aur --scan
```

### Пісочниця

Збірки **AUR та ABS** виконують `pkgver()` / `prepare()` / `build()` / `check()` / `package()` усередині [`bwrap`](https://github.com/containers/bubblewrap). Це обов'язково; відмовитися можна лише явним `--no-sandbox`. Якщо `bubblewrap` не встановлено, збірка відхиляється.

- порожній tmpfs поверх `/home` і `/run` (немає справжнього `$HOME`, сесійної D-Bus, ssh-agent, gpg-agent);
- `--clearenv` плюс невеликий білий список (`PATH`, `TERM`, `LANG`, каталоги призначення збірки, …);
- копія в'язки ключів GnuPG лише з публічними ключами;
- PKGBUILD і `*.install` монтуються повторно лише для читання;
- запис лише в каталог збірки (+ налаштовані `PKGDEST` / `SRCDEST` / …);
- мережа за замовчуванням спільна, інші простори імен вимкнено;
- `fakeroot` у `package()` працює у вкладеному `bwrap` з uid 0;
- seccomp-denylist (ptrace, kexec, завантаження/вивантаження модулів, mount/umount, pivot_root, reboot, swapon/swapoff, iopl/ioperm, bpf, userfaultfd, …).

| Прапор | Ефект |
|--------|-------|
| `--no-sandbox` | Вимкнути bwrap; звичайний `makepkg -si` (також пропускає підтвердження безаудитного fallback'у) |
| `--unshare-net-build` | Мережа лише для завантаження і `prepare()`; `build()`/`check()`/`package()` — без мережі |

Встановлення залежностей (`pacman -S`) і фінальний `pacman -U` виконуються **поза** пісочницею — вони не запускають код PKGBUILD.

### Перевірка готових архівів

Третій бар'єр перед встановленням: готовий `*.pkg.tar.*` (результат збірки або `--usepkg`) перевіряється через `bsdtar`:

- setuid/setgid-бінарники;
- скрипти `.INSTALL` (повторно скануються сканером PKGBUILD);
- чутливі шляхи: хуки alpm, юніти systemd, sudoers, udev, tmpfiles.d, binfmt.d, pam.d, dbus system.d, dispatcher.d у NetworkManager, environment.d, sysusers.d, `ld.so.preload` тощо;
- path traversal і symlink-імена членів архіву.

На TTY потрібне підтвердження навіть без `--ask`; у неінтерактивному режимі — попередження і продовження. Збій аудиту `bsdtar` зупиняє встановлення, а не веде до встановлення без аудиту.

### Єдиний fallback із підтвердженням

PKGBUILD, чиї залежності не можна розв'язати статично (немає `.SRCINFO` і динамічний `depends=(…)`), — єдиний шлях збірки, що закінчується голим `makepkg -si` без пісочниці й аудиту. Мовчки він не запускається: в інтерактиві — явне питання Так/Ні для цього пакунка, у `--noconfirm` — відмова (fail-closed). `--no-sandbox` це питання пропускає: це свідома відмова від пісочниці на весь запуск.

### Інше посилення

- Текст новин Arch очищується (ANSI/bidi не потрапляють у термінал чи `news.state`).
- Файли конфігу, масок, env і world відкриваються з `O_NOFOLLOW`, тож символьне посилання не можна підсунути через гонку.
- Rust-залежності перевіряються через [`cargo deny`](https://github.com/EmbarkStudios/cargo-deny) (`make deny`).

<details>
<summary><h3>Перевірка сканера (після встановлення)</h3></summary>

```bash
git clone https://github.com/Undercat037/aura-emerge.git
cd aura-emerge
emerge --usepkgbuild ./scaner-test --ask
```
</details>

<details>
<summary><h2>Прапори</h2></summary>

| Прапор | Опис |
|------|------|
| `-s`, `--search` | Пошук пакунків |
| `-S`, `--searchdesc` | Пошук за описом |
| `--search-all` | З `-s`: показати те саме ім'я з кожного репозиторію (як `pacman -Ss`) |
| `-sv` | Докладна інформація про пакунок |
| `-v`, `--verbose` | Докладний вивід / докладна інформація під час пошуку (`-sv`) |
| `-u`, `--update` | Повне оновлення системи (world не читає) |
| `--devel` | З `-u`: заодно перезібрати `-git`/`-hg`/`-svn`/`-bzr`, чий upstream пішов уперед |
| `--check-devel` | Показати devel-пакунки зі змінами в upstream (без перезбірки) |
| `-C`, `--unmerge` | Видалити пакунки (unmerge) |
| `-c`, `--depclean` | Видалити сиріт (depclean) |
| `--show-protected` | З `-c`: показати сиріт, що лишилися лише тому, що є у world |
| `-p`, `--pretend` | Pretend (пробний прогін) |
| `-a`, `--ask` | Запитати перед дією |
| `--sudoloop` | Тримати мітку часу sudo «теплою» (оновлення кожні 60 с) весь запуск |
| `-1`, `--oneshot` | Встановити як залежність (у world не потрапляє) |
| `-n`, `--noreplace` | Не перевстановлювати, якщо вже є (та сама або новіша версія) |
| `--with-optdeps` | Заодно встановлювати опційні залежності пакунків з офіційних репозиторіїв |
| `--aur` | Примусово лише AUR (залежності лише з AUR завжди збираються рекурсивно) |
| `--repos` | Лише офіційні репозиторії (AUR не чіпати) |
| `--abs` | Збірка з ABS (`pkgctl`) у пісочниці bwrap |
| `--no-sandbox` | Вимкнути bwrap для AUR/ABS; звичайний makepkg без ізоляції (також пропускає підтвердження fallback'у) |
| `--unshare-net-build` | Без мережі під час build()/check()/package() (у `prepare()` мережа лишається) |
| `--skippgp` | Пропустити перевірки PGP під час збірок ABS (`--skippgpcheck`) |
| `--autopgp` | Автоімпорт відсутніх PGP-ключів перед збіркою `--abs` |
| `--edit` | Редагувати PKGBUILD в `$EDITOR` перед збіркою (лише для запитаного пакунка); після збереження перегенерує `.SRCINFO` |
| `--skip-srcinfo-regen` | З `--edit`: не перегенеровувати `.SRCINFO` |
| `--pkgbuild-view` | Показати/порівняти PKGBUILD і підтвердити перед збіркою (лише для запитаного пакунка; у разі відмови пропонує `$EDITOR`) |
| `--usepkgbuild <PATH>` | Зібрати й встановити локальний каталог із PKGBUILD (сканер + bwrap); у world записується як `local/`, якщо немає `-1` |
| `--usepkg <FILE…\|DIR>` | Встановити локальні архіви `.pkg.tar.*` через аудит + helper `-U` однією транзакцією (залежності між ними розв'язуються разом; каталог = усі архіви в ньому, наприклад результат `-f --outdistfiles`); у world записуються як `local/`, якщо немає `-1` |
| `-O` / `--nodeps` | З `--usepkg`: встановити рівно один файл-архів без перевірки залежностей (`pacman -Udd`) |
| `--scan` | Лише аудит PKGBUILD/.install, без збірки/встановлення; ненульовий код при знахідках (AUR або `--usepkgbuild`) |
| `--sync` | Синхронізувати бази пакунків |
| `--refresh` | Примусовий `-Syy` (з `--sync` або `-u`) |
| `--info` | Інформація про систему, як `emerge --info` у Gentoo |
| `--pkginfo <pkg…>` | Відомості про встановлений пакунок (`-v` перелічує файли) |
| `--news [N\|all]` | Новини Arch Linux: без аргументу - список, `N` - прочитати запис, `all` - позначити все прочитаним |
| `--select` | Додати у world без встановлення |
| `--deselect` | Прибрати зі world без видалення |
| `--check-world` | Показати записи world, яких немає в системі або які встановлені не явно |
| `--sync-explicit` | Позначити як залежності явно встановлені пакунки, яких немає у world, в `@set` зі world та в `package.profile` (щоб `-c` їх бачив). Окремо: виконується одразу (`-p` показує список, `-a` питає). З встановленням/оновленням: після успішного завершення. Можна в `EMERGE_DEFAULT_OPTS` |
| `--prune` | Видалити пакунки, яких немає у world |
| `--localrepo-clean` | Прибрати з `ae-aur` / `ae-abs` / `ae-local` пакунки, яких уже немає в системі (`FEATURES="localrepo"`; `-p` показує, `-a` питає) |
| `--regen` | Перестворити кеш метаданих пакунків |
| `--regen-world` | Заново визначити префікси репозиторіїв у world |
| `--regen-sets <SET>` | Заново визначити префікси у власному наборі (`@game-kit` або `game-kit`) |
| `--regen-sort` | З `--regen-sets`: заодно відсортувати файл (коментарі та групування зникають) |
| `--regen-world-from-explicit` | Міграція: додати у world явно встановлені пакунки (`pacman -Qeq`), яких немає у world, в `@set` зі world та в `package.profile` (одноразово). `-p` лише показує, `-a` питає |
| `--err-install` | З `@world`: заодно встановлювати записи `Err/` (джерело невідоме) |
| `--resume` | Продовжити перервану транзакцію |
| `--skipfirst` | З `--resume`: пропустити перший пакунок |
| `--undo` | Скасувати останнє встановлення/видалення/depclean (один крок; `-u` не відкочується) |
| `--clean-source-cache` | Очистити постійний кеш джерел VCS і вийти |
| `--batchinstall <FILE>` | Масове встановлення з текстового списку (формат як у власного набору) |
| `--list-sets` | Список доступних `@наборів` (використовується й автодоповненням) |
| `--exclude <ATOM>` | Виключити пакунок із цього запуску (повторюваний/через кому) |
| `--keep-going` | Не зупиняти пакетну операцію на першій помилці; збої - у підсумку наприкінці |
| `--revdep-rebuild` | Перезібрати пакунки, чиї бінарники лінкуються з відсутньою бібліотекою |
| `-t`, `--tree` | Показати план як дерево залежностей |
| `-D`, `--deep[=N]` | З `-t`: вкладеність на N рівнів, без значення - на всі |
| `--jobs <N>` | Макс. паралельних встановлень з офіційних репозиторіїв (за замовчуванням 1) |
| `--jobsa <N>` | Макс. паралельних збірок AUR/ABS (за замовчуванням 1) |
| `--load-average <X>` | Не запускати нові збірки, доки середнє навантаження за хвилину вище за X |
| `--nospinner` | Без живого рядка Jobs |
| `--color <y\|n\|auto>` | Примусово ввімкнути або вимкнути колір |
| `--quiet-build=n` | Те саме, що `--debug` (написання Gentoo) |
| `--list-profiles` | Список профілів у стилі Portage (як `eselect profile list`) |
| `--set-profile <N\|PATH>` | Задати `/etc/portage/make.profile`: індекс із `--list-profiles` або шлях |
| `--aeforce` | AE Force: переносити файли, що заважають встановленню, до `<file>.aeforce` без питання |
| `--noaefile` | AE Force: не лишати файли `.ae*` (старий зберегти, новий відкинути; з `--aeforce`: відкинути старий) |
| `--aelist` | Список файлів, залишених AE Force (`.aenew` / `.aeforce` / `.aepark`) |
| `--aeclean [new\|old\|park\|all]` | Видалити файли AE Force після питання (без аргументу = `all`) |
| `--ignore-default-opts` | Ігнорувати `EMERGE_DEFAULT_OPTS` з make.conf на цей запуск |
| `--debug` | Показувати вивід makepkg/компілятора наживо (також `AE_DEBUG=1`); за замовчуванням він прихований заради читабельності рядків `>>>`, а при падінні збірки збережений лог усе одно виводиться |
| `--log <PATH>` | Записати лог сесії (команда, рядки emerge.log, повний вивід збірки) для баг-репортів - `| tee` не потрібен; лише дозапис: наявний вміст не обрізається і не перезаписується |
| `-B` / `--buildpkgonly` | Лише архіви, без встановлення (офіційні => CacheDir; AUR/ABS/локальні => збірка) |
| `-f` / `--fetchonly` | Лише завантаження, все дерево залежностей: офіційні => архіви в CacheDir (разом із залежностями); AUR/ABS => clone + джерела, перевірені в пісочниці (`makepkg --verifysource`, у SRCDEST), для пакунка та його AUR/ABS-залежностей, офіційні залежності - архівами. Без збірки й встановлення |
| `--outdistfiles <DIR>` | З `-B`/`-f`: скопіювати архіви (разом із залежностями) в DIR; checkout'и AUR/ABS лінкуються туди ж |
| `-N`/`--newuse`, `-e`/`--emptytree`, `-q`, `-o`, `-l`, `-A`, `--onlydeps`, `--complete-graph`, `--changed-use`, `--backtrack`, `--columns`, `--newrepo`, `--reinstall`, `--with-bdeps`, `--metadata`, `--clean`, `--config`, `--noconfmem`, `--verbose-conflicts` | Сумісність із Gentoo: приймаються, нічого не роблять |
| `-h`, `--help` | Довідка |
| `-V`, `--version` | Версія |

Сторінка посібника `emerge(1)` генерується з CLI (`emerge --gen-manpage`), тому не може розійтися з `--help`.
</details>

## Автор / Ліцензія
Copyright (C) 2026 **Undercat037**
Ліцензія GPL-3.0
