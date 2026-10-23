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

**Залежності:** `git`, `sudo`, `bubblewrap` (пісочниця збірки для AUR/ABS), `pacman` (libalpm, makepkg)
**Опційно:** `devtools` (підтримка `--abs`), `gnupg` (`--autopgp`, автоімпорт PGP-ключів)

## Використання

```bash
# Встановлення
emerge neovim                       # офіційні репозиторії, у разі невдачі - AUR
emerge neovim-git --aur             # лише AUR (залежності лише з AUR збираються рекурсивно)
emerge neovim --repos               # лише офіційні репозиторії, AUR не чіпати
emerge neovim --abs                 # збірка з ABS (pkgctl) у пісочниці
emerge aur/neovim-git abs/nano cachyos-extra-v3/neovim   # джерело для кожного пакунка - префіксом
emerge -p neovim                    # пробний прогін
emerge -a neovim                    # запитати перед дією
emerge -1 htop                      # встановити як залежність (у world не потрапляє)
emerge -at aur/setrixtui abs/nano @fonts --jobsr=7 --jobsa=2   # різні джерела, паралельні jobs

# Видалення
emerge -C neovim                    # видалити пакунок (unmerge)
emerge -c                           # видалити сиріт (depclean)
emerge -cp --show-protected         # показати сиріт, а також тих, що залишені через world
emerge --prune                      # видалити пакунки, яких немає у world

# Оновлення
emerge --sync                       # оновити бази пакунків
emerge -u                           # повне оновлення (репозиторії + AUR)
emerge -u --devel                   # ...і перезібрати -git/-hg/-svn/-bzr, чий upstream пішов уперед
emerge -u --exclude linux --keep-going
emerge -uat @world --sync           # синхронізація, потім оновлення (запитає)

# World і набори
emerge @world                       # встановити все зі world, чого ще немає
emerge @game-kit neovim             # власний набір + звичайний пакунок
emerge --select neovim              # додати у world без встановлення
emerge --deselect neovim            # прибрати зі world без видалення

# Пошук та інформація
emerge -s neovim                    # пошук: репозиторії, потім ABS, потім AUR
emerge -S neovim                    # пошук за описом
emerge -sv neovim                   # докладна інформація про пакунок
emerge -s abs/nano                  # пошук лише в ABS (префікси aur/ і <repo>/ теж працюють)
emerge -s emacs --repos             # лише офіційні репозиторії
emerge --info                       # інформація про систему
emerge --news                       # новини Arch Linux

# Безпека
emerge neovim-git --aur --pkgbuild-view      # показати/порівняти PKGBUILD перед збіркою
emerge neovim-git --aur --edit               # спершу відредагувати PKGBUILD
emerge --scan neovim-git --aur               # лише аудит, без збірки
emerge --install-pkgbuild ~/src/my-pkg       # зібрати локальний PKGBUILD через сканер і пісочницю

# Відновлення
emerge --resume                     # продовжити перервану транзакцію
emerge --undo                       # скасувати останнє встановлення/видалення/depclean
emerge --revdep-rebuild             # полагодити бінарники зі зламаними посиланнями на бібліотеки
emerge --list-profiles             # профілі (також: eselect profile list)
emerge --aelist                     # файли, залишені AE Force (.aenew / .aeforce / .aepark)
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
EMERGE_DEFAULT_OPTS="--jobsr=8 --tree --deep"

# Кольори рядків статусу (ролі: ok, warn, error, info, special).
# #RRGGBB або імена (red, bright-cyan, …). Останнє згадування ролі перемагає.
COLORS="ok=#FFB3AE warn=#E7BDB9 error=#FDB1AC info=#E7BDB9 special=#cba6f7"

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

**Ключі:** `EMERGE_DEFAULT_OPTS`, `COLORS`, `FEATURES`, `CFLAGS`, `CXXFLAGS`, `CPPFLAGS`, `LDFLAGS`, `RUSTFLAGS`, `MAKEFLAGS`, `NINJAFLAGS`, `BUILDENV`, `OPTIONS`.
**Синтаксис:** як у bash - `KEY="..."` або `KEY=(a b c)`. Рядки та списки взаємозамінні, значення розкриваються оболонкою (`$(nproc)` і `$CFLAGS` працюють; одинарні лапки залишать `$` як є).

`COLORS` задає палітру для статусного виводу: токени `role=colour`. Ролі: `ok`, `warn`, `error`, `info`, `special`. Колір — `#RRGGBB` або ім'я (`red`, `bright-cyan`, …). Невідомі ролі й кольори пропускаються з попередженням.

Токени `FEATURES` (`-name` вимикає токен) перекладаються в аналоги makepkg. Те, що ви самі вказали в `BUILDENV`/`OPTIONS`, головніше.

| `FEATURES`   | makepkg             |
|--------------|---------------------|
| `ccache`     | `BUILDENV` `ccache` |
| `distcc`     | `BUILDENV` `distcc` |
| `test`       | `BUILDENV` `check`  |
| `nostrip`    | `OPTIONS` `!strip`  |
| `splitdebug` | `OPTIONS` `debug`   |

`EMERGE_DEFAULT_OPTS` приймає лише прапори-модифікатори (`--ask`, `--verbose`, `--devel`, `--tree`, `--deep`, `--jobsr=N`, `--jobsa=N`, `--searchdesc`, `--exclude=<atom>`, ...), тож перетворити кожен запуск на `--unmerge` не можна. Конфліктні пари відхиляються, якщо задано обидва: `--aur`/`--abs`, `--aur`/`--repos`, `--no-sandbox`/`--unshare-net-build`, `--skippgp`/`--autopgp`, `--select`/`--deselect`. Якщо значення з конфігу конфліктує з командним рядком, воно відкидається. `--abs` і `--repos` **не** конфліктують.

`emerge --info` показує, що було прочитано, і підсумкове значення кожного перевизначення. [**→ Приклад make.conf**](misc/make.conf)

### `package.mask`

Пакунки, які ця машина не повинна встановлювати ніколи: перевірка спрацьовує навіть для транзитивних залежностей з AUR і навіть коли пакунок запитано за іменем. Замаскований пакунок жорстко зупиняє запит - без питань і без прапора для обходу.

`/etc/portage/package.mask` - файл або каталог (читається кожен звичайний файл у порядку імен; файли з крапки пропускаються; символьні посилання відхиляються). Один запис на рядок, дозволено `*` і префікс репозиторію, наприкінці можна додати `# коментар`.

Префікс репозиторію враховується всюди, де джерело відоме: у плані встановлення (`extra/nano` спрацьовує лише для репозиторію `extra`, `aur/foo-git` - для AUR), у провізіонінгу world, в утриманнях `--select`/`-R`/`-u` та в `package.protect`/`package.freeze`/масках профілю. Версійні атоми Portage (`=cat/foo-1.2`, `>=cat/foo`) не підтримуються - зіставлення йде за назвою пакунка; такі записи виводяться попередженням і пропускаються, а не мовчки ніколи не збігаються.

```
ayugram-desktop-bin            # конкретний пакунок, з будь-якого джерела
aur/*-bin                      # ніколи не встановлювати -bin пакунки з AUR
*-git                          # на цій машині без devel-пакунків
extra/nano                     # конкретний пакунок з конкретного репозиторію
```

Це не те саме, що `--exclude`: маска діє постійно, `--exclude` - на один запуск.

### `package.protect`

`/etc/portage/package.protect` (файл або каталог файлів) перелічує пакунки, які не можна видаляти. Синтаксис як у `package.mask` (необов'язковий префікс `repo/`, глоби `*`, коментарі `#`). Діє на `-C`/`--unmerge`, `-c`/`--depclean`, `--prune` та видалення через конфлікти. `--deselect` і далі може прибрати ім'я зі world, але пакунок залишиться встановленим, і depclean його не чіпатиме. Пакунки `@system` з профілю захищені так само.

### `package.freeze`

`/etc/portage/package.freeze` (і необов'язковий `EMERGE_FREEZE` у `make.conf`) утримує пакунки на встановленій версії під час `-u` / sysupgrade, включно з перезбірками AUR/ABS. Суворіше за `IgnorePkg` у pacman: діє і на вибірковий `emerge -u pkg`.

### Профілі

`/etc/portage/make.profile` - символічне посилання на каталог профілю (розкладка Portage). `--list-profiles` показує дерево з `/var/db/repos/portage/profiles`; `parent` виду `repo:path` шукається в `/var/db/repos/<repo>/profiles/`. Профіль може задавати `parent`, `package.profile` (`*` = стартовий набір `@system`; стара назва `packages` теж читається), `package.mask`, `package.protect` і `make.defaults`. Ланцюжки `parent` розгортаються вглиб. `emerge @system` встановлює системний набір профілю; усі записи `package.profile` захищені від depclean так само, як записи world. Список і перемикання:

```
emerge --list-profiles
sudo emerge --set-profile 12
# або: sudo ln -sf /usr/bin/emerge /usr/bin/eselect && eselect profile set 12
```

`ln -sf /usr/bin/emerge /usr/bin/eselect` перетворює бінарник на мінімальний `eselect` (лише `eselect profile list|set|show`). Для `--list-profiles` і `--set-profile` шим не потрібен.


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

### Прогрес і «живість» збірки

Рядки стадій показують відсоток плану (`>>> Emerging (3 of 12, 16%) …`). У футері Jobs: `Jobs: N of M complete (P%)`. Під час тихої збірки makepkg/компілятора кожні 30 с друкується heartbeat з pid і RSS (`still building … — process alive`); якщо процес убито сигналом (OOM/SIGKILL), це повідомляється явно, а не виглядає як помилка компілятора.


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

### Паралельні jobs

`--jobsr=N` встановлює до N пакунків з офіційних репозиторіїв одночасно, `--jobsa=N` - до N збірок AUR/ABS (за замовчуванням 1 і 1; `--jobs` - псевдонім `--jobsr`). Значення за замовчуванням можна задати в `EMERGE_DEFAULT_OPTS`. `--load-average=X` не запускає нові збірки, доки середнє навантаження за хвилину вище за X (але ніколи, якщо більше нічого не працює); `--nospinner` прибирає живий рядок Jobs.

```bash
emerge -at aur/setrixtui cachyos-extra-v3/neovim abs/nano @fonts --jobsr=7 --jobsa=2
```

```
>>> Jobs: 26 of 26 complete  2m53s                         Load avg: 2.49, 3.11, 2.17
```

Якщо helper відхилив один пакунок із пачки (наприклад, він замаскований), пачка повторюється по одному пакунку, щоб усе інше все одно встановилося; з `--keep-going` збої виводяться підсумком у кінці, а `--resume` повторює лише їх.

### Пошук

`-s` шукає в офіційних репозиторіях, потім в ABS, потім в AUR. Префікс обирає джерело: `-s aur/<name>` - лише AUR, `-s abs/<name>` - ABS, `-s <repo>/<name>` - спочатку цей репозиторій; `--repos` обмежує пошук офіційними репозиторіями. Замасковані пакунки позначаються `[ Masked ]`. `-sv` додає встановлену версію, а для пакунків AUR - мейнтейнера, голоси, популярність, домашню сторінку, дату оновлення та ліцензію.

```bash
emerge -sv ttf-comic-sans
emerge -s emacs --repos
emerge -s abs/nano
```

### Видалення пакунків

`-C` видаляє вказані пакунки, `-c` видаляє сиріт, `--prune` видаляє все, чого немає у world. `-C` друкує, що `selected`, `protected` і `omitted`; потім `-C` і `-c` чекають 5 секунд (`Unmerging in: 5 4 3 2 1`, Ctrl-C перериває), якщо тільки `-a` уже не запитав вас або не задано `-p`. Після `-C` імена також прибираються зі world.

`-c` ніколи не видаляє запис world, члена набору, записаного у world, запис `package.protect` і пакунок `@system` з профілю. Він повідомляє, скільки пропущено; `--show-protected` перелічує їх.

```bash
emerge -cp --show-protected
emerge -c
emerge --unmerge --ask linux-zen-headers
```

Видалене через `-c` можна повернути командою `--undo` (див. нижче).

### Конфлікти (блокери)

Встановлені пакунки, що блокують вхідний, показуються до початку встановлення, як блокери Portage:

```
[ebuild  N    ] pipewire-jack-1:1.4.2-1
[conflict     ] jack2-1.9.22-1 ("jack2" is blocking pipewire-jack-1:1.4.2-1)
```

Перевірка охоплює всі джерела запуску: план з репозиторіїв (libalpm), пакунки AUR та їхні залежності з AUR, пакунки ABS (за їхнім `.SRCINFO`: `provides`/`conflicts`) і голий `@world`. Джерела також перевіряються **одне проти одного**: якщо два пакунки одного запуску блокують один одного (наприклад, пакунок з репозиторію і пакунок з AUR), це показується як `[blocks]`, і нічого не встановлюється. Для встановлених блокерів потрібен один `Yes` (питається завжди, з `--ask` або без); спершу перелічуються пакунки, яким вони потрібні, а записи `package.protect` не видаляються ніколи. Те, що попередня перевірка побачити не змогла (наприклад, немає мережі для `.SRCINFO`), усе одно ловиться під час встановлення пакунка.

### Хуки

libalpm виконує кожен `*.hook` під час встановлення, видалення, `-U` та `-Su`, як і pacman (depmod, DKMS, dracut/mkinitcpio, кеші fontconfig та іконок, `grub-mkconfig`, …). Каталоги; за збігу імен перемагає ранніший:

```
/usr/share/libalpm/hooks/    хуки з пакунків
/etc/pacman.d/hooks/         хуки адміністратора
/etc/portage/hooks/          власні хуки aura-emerge (той самий формат, alpm-hooks(5))
```

Усі три реєструються завжди, навіть якщо в `pacman.conf` немає `HookDir`. Хід виконання друкується як `>>> Running pre-transaction hooks...` / `>>> (2 of 4) Install DKMS modules`.

### AE Force (конфлікти файлів)

Якщо встановлення перезаписало б файл, яким не володіє жоден пакунок, aura-emerge питає `AE Force: Create .aenew for conflict files?` «Так» залишає ваш файл на місці, а версію з пакунка зберігає як `<file>.aenew`.

| Прапор | Що відбувається |
|--------|-----------------|
| `--aeforce` | без питання; ваш старий файл переноситься до `<file>.aeforce`, новий займає його місце |
| `--noaefile` | без питання; файлів `.ae*` не лишається: старий збережено, новий відкинуто (з `--aeforce`: старий відкинуто, новий збережено) |

`<file>.aepark` лишається після перерваного встановлення; якщо ім'я зайняте, додається суфікс `.N`. `emerge --aelist` перелічує всі такі файли, `emerge --aeclean [new|old|park|all]` видаляє їх після питання (просто `--aeclean` = `all`).

### `--info`

`emerge --info` друкує версію (і версію libalpm, з якою зібрано), ядро, CPU/пам'ять/swap, чинні `--jobsr`/`--jobsa`, кожен репозиторій у порядку `pacman.conf` із дзеркалом, значення з `makepkg.conf` і `make.conf` (і які перевизначення перемогли), профіль, розміри mask/world та підсумки журналу merge.

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

`SRCDEST` за замовчуванням - `~/.cache/aura-emerge/sources` (або `$XDG_CACHE_HOME/aura-emerge/sources`) для **усіх** збірок AUR/ABS, не лише VCS: makepkg повторно використовує завантажені tarball'и і робить інкрементальний fetch. Чекаути PKGBUILD AUR/ABS живуть у `~/.cache/aura-emerge/build/{aur,abs}/` і оновлюються на місці (`git fetch` / `git pull`), а не стираються кожен запуск. Якщо `SRCDEST` задано в `makepkg.conf`, використовується він. Очистити кеш джерел: `emerge --clean-source-cache`.

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

`/etc/portage/sets/<name>` (або `<name>.set`) викликається як `@<name>`. Один пакунок на рядок, коментарі `#` дозволені. Рядки виду `@other` вкладають інший набір (розгортається рекурсивно, цикли безпечні). Встановлення `@name` записує `@name` у world, тож члени набору захищені від depclean, доки їх не приберуть із файлу набору.

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

Відкочує останнє успішне встановлення, видалення або depclean (один крок історії):

- **після встановлення** - видаляє лише пакунки, що в цьому запуску були новими, і повертає встановлені пакунки, видалені як блокери;
- **після видалення** - перевстановлює із записаного джерела (`Err/`/`abs/` лише перелічуються);
- **після depclean** - перевстановлює видалених сиріт **як залежності** (до world не додаються): пакунки з репозиторіїв - з репозиторіїв, локальні збірки - через AUR, якщо він їх ще містить. Версії записуються: якщо в репозиторіях тепер інша, вам про це скажуть; пакунок, якого більше ніде немає, перелічується з рядком `pacman -U` для файлу з кешу. Якщо частина пакунків не встала, у збереженому стані лишаються тільки вони, і `--undo` можна запустити знову;
- **після `-u`** - не підтримується (немає знімка версій).

Стан зберігається в `/etc/portage/lastaction.state` (`--undo`) і `/etc/portage/resume.state` (`--resume`).

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
2. **Рядкові евристики:** `chmod 777`, «голі» IPv4, сторонні менеджери пакунків (`npm`/`pnpm`/`bun`/`yarn`/`pip`/`gem install …`, лише віддалені `cargo install`/`go install`), `bash <(curl …)`, `curl … | python3/perl/node/…`, розпакування одразу в шелл (`gunzip`/`zstd -d`/`unzip -p`/`tar -xO | sh`), канали `/dev/tcp`//dev/udp`, heredoc'и, що скармлюються шеллу (або з fetch/decode у тілі), paste-сайти, `.onion`, перевірки на антипісочницю, відомі IOC кампаній (Atomic Arch, openconnect-sso, `compromised.txt` 2018 року, …).

Знахідки бувають `Suspicious` або `ConfirmedIoc`. Спрацювання не блокує жорстко: ви отримуєте файл/рядок, посилання на cgit і `Continue anyway? [y/N]`.

**Допоміжні файли:** коли checkout лежить на диску (збірка або `--install-pkgbuild <PATH> --scan`), `Makefile` (`Makefile`/`makefile`/`GNUmakefile`) та shell-скрипти, на які посилається PKGBUILD (`*.sh`, `*.bash`, `*.zsh`, `*.fish`), теж перевіряються построково: `curl|sh`, base64/hex у shell, «голі» IP та решта shell-IOC. У знахідці вказано файл і рядок. `--scan <pkg> --aur` бачить лише те, що віддає cgit (`PKGBUILD` і `.install`). Патчі поки не скануються.

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
| `--no-sandbox` | Вимкнути bwrap; звичайний `makepkg -si` (також пропускає підтвердження безаудитного fallback'у) |
| `--unshare-net-build` | Мережа лише для завантаження/`prepare()`; `build()`/`check()`/`package()` - без мережі |

Якщо `bubblewrap` не встановлено, збірка відхиляється.

**Третій бар'єр перед встановленням:** готовий `*.pkg.tar.*` перевіряється через `bsdtar` на setuid/setgid, скрипти `.INSTALL` і чутливі шляхи (хуки alpm, юніти systemd, sudoers, udev, tmpfiles.d, binfmt.d, pam.d, dbus system.d, dispatcher.d у NetworkManager, environment.d, sysusers.d, …). `.INSTALL` повторно сканується сканером PKGBUILD. З `-a` потрібно підтвердити; з `--noconfirm` - попередження і продовження; невдалий аудит bsdtar зупиняє встановлення замість безаудитного.

**Єдиний fallback, що підтверджується:** PKGBUILD, чиї залежності неможливо розв'язати статично (немає `.SRCINFO` і динамічний `depends=(…)`), - єдиний шлях збірки, куди пакунок може завести сам автор і який закінчується голим `makepkg -si` без пісочниці й аудиту. Мовчки він не запускається: в інтерактиві - явне питання Так/Ні для цього пакунка; у `--noconfirm`-запуску - відмова (fail-closed). `--no-sandbox` пропускає це питання - це усвідмана відмова від пісочниці на весь запуск.

Встановлення залежностей (`pacman -S`) і фінальний `pacman -U` виконуються **поза** пісочницею - вони не запускають код PKGBUILD.

### Привілеї

Фронтенд ніколи не працює від root. Якщо запуск від root (`sudo emerge …`, `doas`, `pkexec`, `su -`), він спершу стартує root-helper, потім назавжди знижує права до користувача, що викликав (береться з `AE_USER`, `SUDO_UID`, `DOAS_USER`, `PKEXEC_UID` або audit login uid; якщо нічого не знайдено - зупинка). Під час запуску від звичайного користувача helper стартує через `sudo` при першій потребі. Резолвер, збірки AUR/ABS, пісочниця та всі кеші працюють без привілеїв; робота через libalpm (install/remove/sync/upgrade/`-U`, перевірка підписів) іде через helper (`--ae-service`), який перевіряє кожен запит і сам застосовує `package.mask` (`helper refused: … masked by …`).

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
| `--install-pkgbuild <PATH>` | Зібрати й встановити локальний каталог із PKGBUILD (сканер + bwrap); у world записується як `Err/`, якщо немає `-1` |
| `--scan` | Лише аудит PKGBUILD/.install, без збірки/встановлення; ненульовий код при знахідках (AUR або `--install-pkgbuild`) |
| `--sync` | Синхронізувати бази пакунків |
| `--refresh` | Примусовий `-Syy` (з `--sync` або `-u`) |
| `--info` | Інформація про систему, як `emerge --info` у Gentoo |
| `--news [N\|all]` | Новини Arch Linux: без аргументу - список, `N` - прочитати запис, `all` - позначити все прочитаним |
| `--check-news [N\|all]` | Псевдонім `--news` |
| `--select` | Додати у world без встановлення |
| `--deselect` | Прибрати зі world без видалення |
| `--check-world` | Показати записи world, яких немає в системі або які встановлені не явно |
| `--sync-explicit` | Позначити як залежності явно встановлені пакунки, яких немає у world, в `@set` зі world та в `package.profile` (щоб `-c` їх бачив). Окремо: виконується одразу (`-p` показує список, `-a` питає). З встановленням/оновленням: після успішного завершення. Можна в `EMERGE_DEFAULT_OPTS` |
| `--prune` | Видалити пакунки, яких немає у world |
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
| `--jobsr <N>` | Макс. паралельних встановлень з офіційних репозиторіїв (за замовчуванням 1) |
| `--jobsa <N>` | Макс. паралельних збірок AUR/ABS (за замовчуванням 1) |
| `--jobs <N>` | Псевдонім `--jobsr` (у стилі Portage) |
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
| `-N`/`--newuse`, `-e`/`--emptytree`, `-q`, `-O`, `-o`, `-k`, `-K`, `-g`, `-G`, `-b`, `-B`, `-f`, `-F`, `-l`, `-A`, `--nodeps`, `--onlydeps`, `--complete-graph`, `--changed-use`, `--backtrack`, `--columns`, `--newrepo`, `--reinstall`, `--with-bdeps`, `--metadata`, `--clean`, `--config`, `--noconfmem`, `--verbose-conflicts` | Сумісність із Gentoo: приймаються, нічого не роблять |
| `-h`, `--help` | Довідка |
| `-V`, `--version` | Версія |

Сторінка посібника `emerge(1)` генерується з CLI (`emerge --gen-manpage`), тому не може розійтися з `--help`.
</details>

## Автор / Ліцензія
Copyright (C) 2026 **Undercat037**
Ліцензія GPL-3.0
