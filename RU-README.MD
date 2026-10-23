<div align="center">
  <img src="misc/logo/slash-logo-128x128.svg" width="128" alt="aura-emerge logo">

# Aura-Emerge Secure Package Manager

[![AE_Site](https://img.shields.io/badge/AE_Site-333333?style=flat-square&logo=bookstack&logoColor=FFFFFF)](https://undercat037.github.io/aura-emerge/)
[![AUR](https://img.shields.io/badge/AUR-333333?style=flat-square&logo=arch-linux&logoColor=1793D1)](https://aur.archlinux.org/packages/aura-emerge)
[![GitHub](https://img.shields.io/badge/GitHub-333333?style=flat-square&logo=github&logoColor=FFFFFF)](https://github.com/Undercat037/aura-emerge)
[![GitLab](https://img.shields.io/badge/GitLab-333333?style=flat-square&logo=gitlab&logoColor=FC6D26)](https://gitlab.com/Undercat037/aura-emerge)

[en](README.MD) | [ua](UA-README.MD) | ru
</div>

> Автономный менеджер пакетов `emerge` в стиле Gentoo для Arch Linux. Ставит пакеты из официальных репозиториев, AUR и ABS; перед сборкой сканирует PKGBUILD на признаки атак на цепочку поставок; ненадёжные шаги сборки выполняет в песочнице `bwrap`.

Пакеты учитываются в файле `world` - установил один раз, отслеживается всегда.

## Установка

```bash
paru -S aura-emerge          # через AUR-хелпер
```

или обычным makepkg:

```bash
git clone https://aur.archlinux.org/aura-emerge.git
cd aura-emerge
makepkg -si
```

<details>
<summary><h3>Сборка из исходников (нестабильно)</h3></summary>

```bash
git clone https://github.com/Undercat037/aura-emerge.git
cd aura-emerge
make install            # или вручную:
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

**Зависимости:** `git`, `sudo`, `bubblewrap` (песочница сборки для AUR/ABS), `pacman` (libalpm, makepkg)
**Опционально:** `devtools` (поддержка `--abs`), `gnupg` (`--autopgp`, автоимпорт PGP-ключей)

## Использование

```bash
# Установка
emerge neovim                       # официальные репозитории, при неудаче - AUR
emerge neovim-git --aur             # только AUR (зависимости только из AUR собираются рекурсивно)
emerge neovim --repos               # только официальные репозитории, AUR не трогать
emerge neovim --abs                 # сборка из ABS (pkgctl) в песочнице
emerge aur/neovim-git abs/nano cachyos-extra-v3/neovim   # источник для каждого пакета - префиксом
emerge -p neovim                    # пробный прогон
emerge -a neovim                    # спросить перед действием
emerge -1 htop                      # поставить как зависимость (в world не попадает)
emerge -at aur/setrixtui abs/nano @fonts --jobsr=7 --jobsa=2   # разные источники, параллельные jobs

# Удаление
emerge -C neovim                    # удалить пакет (unmerge)
emerge -c                           # удалить сирот (depclean)
emerge -cp --show-protected         # показать сирот, а также тех, что оставлены из-за world
emerge --prune                      # удалить пакеты, которых нет в world

# Обновление
emerge --sync                       # обновить базы пакетов
emerge -u                           # полное обновление (репозитории + AUR)
emerge -u --devel                   # ...и пересобрать -git/-hg/-svn/-bzr, чей апстрим ушёл вперёд
emerge -u --exclude linux --keep-going
emerge -uat @world --sync           # синхронизация, затем обновление (спросит)

# World и наборы
emerge @world                       # поставить всё из world, чего ещё нет
emerge @game-kit neovim             # пользовательский набор + обычный пакет
emerge --select neovim              # добавить в world без установки
emerge --deselect neovim            # убрать из world без удаления

# Поиск и информация
emerge -s neovim                    # поиск: репозитории, затем ABS, затем AUR
emerge -S neovim                    # поиск по описанию
emerge -sv neovim                   # подробная информация о пакете
emerge -s abs/nano                  # поиск только в ABS (префиксы aur/ и <repo>/ тоже работают)
emerge -s emacs --repos             # только официальные репозитории
emerge --info                       # информация о системе
emerge --news                       # новости Arch Linux

# Безопасность
emerge neovim-git --aur --pkgbuild-view      # показать/сравнить PKGBUILD перед сборкой
emerge neovim-git --aur --edit               # сначала отредактировать PKGBUILD
emerge --scan neovim-git --aur               # только аудит, без сборки
emerge --install-pkgbuild ~/src/my-pkg       # собрать локальный PKGBUILD через сканер и песочницу

# Восстановление
emerge --resume                     # продолжить прерванную транзакцию
emerge --undo                       # отменить последнюю установку/удаление/depclean
emerge --revdep-rebuild             # починить бинарники с битыми ссылками на библиотеки
emerge --list-profiles             # профили (также: eselect profile list)
emerge --aelist                     # файлы, оставленные AE Force (.aenew / .aeforce / .aepark)
```

Полная таблица флагов - в конце страницы.

## Настройка

### Gentoo → Arch: аналоги `make.conf`

В Arch нет USE-флагов, но компиляцию можно настроить почти так же, как в `/etc/portage/make.conf` у [Gentoo](https://wiki.gentoo.org/wiki/Main_Page) - через `/etc/makepkg.conf`:

| Gentoo (`make.conf`)         | Arch (`makepkg.conf`)         | Назначение                      |
|------------------------------|-------------------------------|---------------------------------|
| `CFLAGS="-O2 -march=native"` | `CFLAGS="-O2 -march=native"`  | Флаги компилятора C             |
| `CXXFLAGS="-O2 -march=..."`  | `CXXFLAGS="-O2 -march=..."`   | Флаги компилятора C++           |
| `RUSTFLAGS="-C opt-level=3"` | `RUSTFLAGS="-C opt-level=3"`  | Флаги компилятора Rust          |
| `MAKEOPTS="-j$(nproc)"`      | `MAKEFLAGS="-j$(nproc)"`      | Параллельные задачи сборки      |
| `USE="lto"`                  | `OPTIONS+=(lto)`              | Оптимизация на этапе компоновки |
| `FEATURES="ccache"`          | `BUILDENV+=(ccache)`          | Кэш сборки                      |
| `PORTDIR` / `PKGDIR`         | `PKGDEST`, `SRCDEST`          | Каталоги артефактов сборки      |

См. [флаги оптимизации GCC](https://gcc.gnu.org/onlinedocs/gcc/Optimize-Options.html), [параметры кодогенерации rustc](https://doc.rust-lang.org/rustc/codegen-options/index.html) и [**пример makepkg.conf**](misc/makepkg.conf).

### `/etc/portage/make.conf`

Собственный конфиг aura-emerge. Переопределяет `/etc/makepkg.conf` для сборок через aura-emerge, не трогая сам файл.

```bash
# Флаги по умолчанию, подставляются перед набранным вами (командная строка главнее).
# Отключить для одного запуска: --ignore-default-opts.
EMERGE_DEFAULT_OPTS="--jobsr=8 --tree --deep"

# Цвета строк статуса (роли: ok, warn, error, info, special).
# #RRGGBB или имена (red, bright-cyan, …). Последнее упоминание роли побеждает.
COLORS="ok=#FFB3AE warn=#E7BDB9 error=#FDB1AC info=#E7BDB9 special=#cba6f7"

# Флаги сборки (применяются через сгенерированный makepkg.conf)
CFLAGS="-march=native -O2 -pipe"
CXXFLAGS="$CFLAGS"
RUSTFLAGS="-C target-cpu=native"
MAKEFLAGS="-j$(nproc)"
NINJAFLAGS="-j$(nproc)"
OPTIONS=(strip docs !debug lto)
BUILDENV=(!distcc color check !sign)

# Переключатели в стиле Portage, переводятся в makepkg
FEATURES="ccache -test"
```

**Ключи:** `EMERGE_DEFAULT_OPTS`, `COLORS`, `FEATURES`, `CFLAGS`, `CXXFLAGS`, `CPPFLAGS`, `LDFLAGS`, `RUSTFLAGS`, `MAKEFLAGS`, `NINJAFLAGS`, `BUILDENV`, `OPTIONS`.
**Синтаксис:** как в bash - `KEY="..."` или `KEY=(a b c)`. Строки и списки взаимозаменяемы, значения раскрываются оболочкой (`$(nproc)` и `$CFLAGS` работают; одинарные кавычки оставят `$` как есть).

`COLORS` задаёт палитру для статусного вывода: токены `role=colour`. Роли: `ok`, `warn`, `error`, `info`, `special`. Цвет — `#RRGGBB` или имя (`red`, `bright-cyan`, …). Неизвестные роли и цвета пропускаются с предупреждением.

Токены `FEATURES` (`-name` выключает токен) переводятся в аналоги makepkg. То, что вы сами указали в `BUILDENV`/`OPTIONS`, главнее.

| `FEATURES`   | makepkg             |
|--------------|---------------------|
| `ccache`     | `BUILDENV` `ccache` |
| `distcc`     | `BUILDENV` `distcc` |
| `test`       | `BUILDENV` `check`  |
| `nostrip`    | `OPTIONS` `!strip`  |
| `splitdebug` | `OPTIONS` `debug`   |

`EMERGE_DEFAULT_OPTS` принимает только флаги-модификаторы (`--ask`, `--verbose`, `--devel`, `--tree`, `--deep`, `--jobsr=N`, `--jobsa=N`, `--searchdesc`, `--exclude=<atom>`, ...), так что превратить каждый запуск в `--unmerge` нельзя. Конфликтующие пары отклоняются, если заданы обе: `--aur`/`--abs`, `--aur`/`--repos`, `--no-sandbox`/`--unshare-net-build`, `--skippgp`/`--autopgp`, `--select`/`--deselect`. Если значение из конфига конфликтует с командной строкой, оно отбрасывается. `--abs` и `--repos` **не** конфликтуют.

`emerge --info` показывает, что было прочитано, и итоговое значение каждого переопределения. [**→ Пример make.conf**](misc/make.conf)

### `package.mask`

Пакеты, которые эта машина не должна ставить никогда: проверка срабатывает даже для транзитивных зависимостей из AUR и даже при запросе по имени. Замаскированный пакет жёстко останавливает запрос - без вопросов и без флага для обхода.

`/etc/portage/package.mask` - файл или каталог (читается каждый обычный файл по порядку имён; файлы с точки пропускаются; символьные ссылки отклоняются). Одна запись на строку, допускаются `*` и префикс репозитория, в конце можно добавить `# комментарий`.

Префикс репозитория учитывается везде, где источник известен: в плане установки (`extra/nano` срабатывает только для репозитория `extra`, `aur/foo-git` - для AUR), в провижининге world, в удержаниях `--select`/`-R`/`-u` и в `package.protect`/`package.freeze`/масках профиля. Версионные атомы Portage (`=cat/foo-1.2`, `>=cat/foo`) не поддерживаются - сопоставление идёт по имени пакета; такие записи выводятся предупреждением и пропускаются, а не молча не совпадают никогда.

```
ayugram-desktop-bin            # конкретный пакет, из любого источника
aur/*-bin                      # никогда не ставить -bin пакеты из AUR
*-git                          # на этой машине без devel-пакетов
extra/nano                     # конкретный пакет из конкретного репозитория
```

Это не то же самое, что `--exclude`: маска действует постоянно, `--exclude` - на один запуск.

### `package.protect`

`/etc/portage/package.protect` (файл или каталог файлов) перечисляет пакеты, которые нельзя удалять. Синтаксис как у `package.mask` (необязательный префикс `repo/`, глобы `*`, комментарии `#`). Действует на `-C`/`--unmerge`, `-c`/`--depclean`, `--prune` и удаления из-за конфликтов. `--deselect` по-прежнему может убрать имя из world, но пакет останется установленным, и depclean его не тронет. Пакеты `@system` из профиля защищены так же.

### `package.freeze`

`/etc/portage/package.freeze` (и необязательный `EMERGE_FREEZE` в `make.conf`) удерживает пакеты на установленной версии при `-u` / sysupgrade, включая пересборки AUR/ABS. Строже, чем `IgnorePkg` в pacman: действует и на выборочный `emerge -u pkg`.

### Профили

`/etc/portage/make.profile` - символьная ссылка на каталог профиля (раскладка Portage). `--list-profiles` показывает дерево из `/var/db/repos/portage/profiles`; `parent` вида `repo:path` ищется в `/var/db/repos/<repo>/profiles/`. Профиль может задавать `parent`, `package.profile` (`*` = стартовый набор `@system`; старое имя `packages` тоже читается), `package.mask`, `package.protect` и `make.defaults`. Цепочки `parent` раскрываются в глубину. `emerge @system` ставит системный набор профиля; все записи `package.profile` защищены от depclean так же, как записи world. Список и переключение:

```
emerge --list-profiles
sudo emerge --set-profile 12
# или: sudo ln -sf /usr/bin/emerge /usr/bin/eselect && eselect profile set 12
```

`ln -sf /usr/bin/emerge /usr/bin/eselect` превращает бинарник в минимальный `eselect` (только `eselect profile list|set|show`). Для `--list-profiles` и `--set-profile` шим не нужен.


### `package.env`

Флаги сборки для отдельных пакетов. Каждая запись - атом и одно или несколько имён env-файлов из `/etc/portage/env/`. Как и `package.mask`, `/etc/portage/package.env` может быть файлом или каталогом.

```
# /etc/portage/package.env/neovim
abs/neovim  lto.conf            # префикс репозитория и '*' работают как в package.mask
aur/*-git   no-lto  fast        # несколько env, применяются в указанном порядке
```

```bash
# /etc/portage/env/lto.conf  (синтаксис make.conf; значимы только ключи сборки)
CFLAGS="-march=znver3 -O2 -pipe"
MAKEFLAGS="-j16"
OPTIONS="strip docs !debug lto"
```

- **Наложение:** `make.conf` → подходящие записи `package.env` → каждый env-файл в указанном порядке. Побеждает слой, который задал ключ последним, и ключ заменяется целиком, а не сливается (`OPTIONS` из env-файла заменяет `OPTIONS` из `make.conf`). Сборка напишет, какой слой победил.
- **Совпадение:** по pkgbase или любому из pkgname.
- **Область:** только сборки AUR/ABS. `EMERGE_DEFAULT_OPTS` в env-файле игнорируется с предупреждением.
- **Ошибки:** символьная ссылка, некорректная запись или отсутствующий env-файл пропускаются с предупреждением (с файлом и строкой); остальное применяется.

## Возможности

### Прогресс и «живость» сборки

Строки стадий показывают процент плана (`>>> Emerging (3 of 12, 16%) …`). В футере Jobs: `Jobs: N of M complete (P%)`. Во время тихой сборки makepkg/компилятора каждые 30 с печатается heartbeat с pid и RSS (`still building … — process alive`); если процесс убит сигналом (OOM/SIGKILL), это сообщается явно, а не выглядит как ошибка компилятора.


### `--exclude` и `--keep-going`

`--exclude <atom>` (можно повторять или перечислять через запятую) исключает пакет из текущего запуска: установки, `-u`, `@world`, `--depclean`/`--prune` (защищает от удаления) и `--revdep-rebuild`.

`--keep-going` не останавливает пакетную операцию (`@world`, `-u`, `--batchinstall`, установку нескольких пакетов) на первой ошибке. Он запоминает сбои, в конце печатает сводку и завершается с ненулевым кодом. Последующий `--resume` повторит только упавшее.

```bash
emerge -u --exclude linux --exclude aur/ayugram-desktop-bin   # обновить всё, кроме этих
emerge -u --keep-going                                        # не останавливаться на одном плохом пакете из AUR
emerge --resume                                               # повторить только упавшее
```

### `-t` / `--tree` и `--deep`

Показывает план (с `-p` или прямо перед реальной установкой) в виде дерева зависимостей. По умолчанию вложен один уровень; `--deep` идёт до самого низа, `--deep=N` - до N уровней.

```bash
emerge -apt noctalia --aur           # ask + pretend + tree
emerge -pt noctalia --aur --deep     # на всю глубину
```

```
[ebuild  N ] noctalia-5.1.0-1.1
[ebuild  N ] `-- libqalculate-5.12.0-1.1
[ebuild  N ]   `-- sdbus-cpp-2.3.1-1.1
```

Зависимость, которая не названа ни в чьём «Depends On» (разрешена через `provides` или лежит глубже `--deep`), всё равно показывается, просто без родителя. Для чистой установки через `--aur` транзитивные зависимости только из AUR на шаге плана пока не разрешаются, поэтому дерево там плоское.

### Параллельные jobs

`--jobsr=N` ставит до N пакетов из официальных репозиториев одновременно, `--jobsa=N` - до N сборок AUR/ABS (по умолчанию 1 и 1; `--jobs` - псевдоним `--jobsr`). Значения по умолчанию можно задать в `EMERGE_DEFAULT_OPTS`. `--load-average=X` не запускает новые сборки, пока средняя нагрузка за минуту выше X (но никогда, если больше ничего не работает); `--nospinner` убирает живую строку Jobs.

```bash
emerge -at aur/setrixtui cachyos-extra-v3/neovim abs/nano @fonts --jobsr=7 --jobsa=2
```

```
>>> Jobs: 26 of 26 complete  2m53s                         Load avg: 2.49, 3.11, 2.17
```

Если helper отклонил один пакет из пачки (например, он замаскирован), пачка повторяется по одному пакету, чтобы всё остальное всё равно установилось; с `--keep-going` сбои выводятся итогом в конце, а `--resume` повторяет только их.

### Поиск

`-s` ищет в официальных репозиториях, затем в ABS, затем в AUR. Префикс выбирает источник: `-s aur/<name>` - только AUR, `-s abs/<name>` - ABS, `-s <repo>/<name>` - сначала этот репозиторий; `--repos` ограничивает поиск официальными репозиториями. Замаскированные пакеты помечаются `[ Masked ]`. `-sv` добавляет установленную версию, а для пакетов AUR - мейнтейнера, голоса, популярность, домашнюю страницу, дату обновления и лицензию.

```bash
emerge -sv ttf-comic-sans
emerge -s emacs --repos
emerge -s abs/nano
```

### Удаление пакетов

`-C` удаляет указанные пакеты, `-c` удаляет сирот, `--prune` удаляет всё, чего нет в world. `-C` печатает, что `selected`, `protected` и `omitted`; затем `-C` и `-c` ждут 5 секунд (`Unmerging in: 5 4 3 2 1`, Ctrl-C прерывает), если только `-a` уже не спросил вас или не задан `-p`. После `-C` имена также убираются из world.

`-c` никогда не удаляет запись world, члена набора, записанного в world, запись `package.protect` и пакет `@system` из профиля. Он сообщает, сколько пропущено; `--show-protected` перечисляет их.

```bash
emerge -cp --show-protected
emerge -c
emerge --unmerge --ask linux-zen-headers
```

Удалённое через `-c` можно вернуть командой `--undo` (см. ниже).

### Конфликты (блокеры)

Установленные пакеты, которые блокируют входящий, показываются до начала установки, как блокеры Portage:

```
[ebuild  N    ] pipewire-jack-1:1.4.2-1
[conflict     ] jack2-1.9.22-1 ("jack2" is blocking pipewire-jack-1:1.4.2-1)
```

Проверка охватывает все источники запуска: план из репозиториев (libalpm), пакеты AUR и их зависимости из AUR, пакеты ABS (по их `.SRCINFO`: `provides`/`conflicts`) и голый `@world`. Источники также проверяются **друг против друга**: если два пакета одного запуска блокируют друг друга (например, пакет из репозитория и пакет из AUR), это показывается как `[blocks]`, и ничего не устанавливается. Для установленных блокеров нужен один `Yes` (спрашивается всегда, с `--ask` или без); сначала перечисляются пакеты, которым они нужны, а записи `package.protect` не удаляются никогда. То, что предварительная проверка увидеть не смогла (например, нет сети для `.SRCINFO`), всё равно ловится при установке пакета.

### Хуки

libalpm выполняет каждый `*.hook` при установке, удалении, `-U` и `-Su`, как и pacman (depmod, DKMS, dracut/mkinitcpio, кэши fontconfig и иконок, `grub-mkconfig`, …). Каталоги; при совпадении имён побеждает более ранний:

```
/usr/share/libalpm/hooks/    хуки из пакетов
/etc/pacman.d/hooks/         хуки администратора
/etc/portage/hooks/          собственные хуки aura-emerge (тот же формат, alpm-hooks(5))
```

Все три регистрируются всегда, даже если в `pacman.conf` нет `HookDir`. Ход выполнения печатается как `>>> Running pre-transaction hooks...` / `>>> (2 of 4) Install DKMS modules`.

### AE Force (конфликты файлов)

Если установка перезаписала бы файл, которым не владеет ни один пакет, aura-emerge спрашивает `AE Force: Create .aenew for conflict files?` «Да» оставляет ваш файл на месте, а версию из пакета сохраняет как `<file>.aenew`.

| Флаг | Что происходит |
|------|----------------|
| `--aeforce` | без вопроса; ваш старый файл переносится в `<file>.aeforce`, новый занимает его место |
| `--noaefile` | без вопроса; файлов `.ae*` не остаётся: старый сохранён, новый отброшен (с `--aeforce`: старый отброшен, новый сохранён) |

`<file>.aepark` остаётся после прерванной установки; если имя занято, добавляется суффикс `.N`. `emerge --aelist` перечисляет все такие файлы, `emerge --aeclean [new|old|park|all]` удаляет их после вопроса (просто `--aeclean` = `all`).

### `--info`

`emerge --info` печатает версию (и версию libalpm, с которой собрано), ядро, CPU/память/swap, действующие `--jobsr`/`--jobsa`, каждый репозиторий в порядке `pacman.conf` с зеркалом, значения из `makepkg.conf` и `make.conf` (и какие переопределения победили), профиль, размеры mask/world и итоги журнала merge.

### Журнал merge

`/var/log/emerge.log` - только дозапись, одна строка на событие. Неудачная запись в журнал никогда не ломает сам merge.

```
2026-09-21 08:51:44  MERGE    aur    noctalia-5.1.0-1.1        (12s)
2026-09-21 09:03:02  MERGE    repo   nano-8.0-1  vim-9.1-1     (4s)
2026-09-21 09:10:05  UNMERGE  -      old-package-2.0-1
```

Пакеты AUR/ABS собираются по одному, поэтому у каждого свой таймер. `pacman -S`/`-Syu` - одна транзакция, так что её строка содержит все атомы и общее время. `--info` показывает итоги (число merge/unmerge, суммарное время сборки).

### Пакетная установка

`--batchinstall <FILE>` ставит пакеты из текстового списка (один атом на строку, комментарии `#` допускаются). Формат тот же, что у [пользовательского набора](#пользовательские-наборы), но файл можно взять откуда угодно - регистрировать не нужно. Сочетается с любыми флагами (`-p`, `-a`, `--aur`, `--abs`, `--repos`, ...).

```bash
emerge --batchinstall pkgs.txt
```

### Просмотр и проверка PKGBUILD

**`--pkgbuild-view`** показывает PKGBUILD прямо перед сборкой и просит подтверждения: diff с последней показанной копией (кэш в `~/.cache/aura-emerge/pkgbuild-view/`) или весь файл в первый раз. При отказе предлагает открыть `$EDITOR`, а не просто пропускает пакет. Работает после `--edit`, так что вы видите свои правки. Применяется только к напрямую запрошенным пакетам AUR/ABS - не к рекурсивным зависимостям из AUR и не к пакетным путям (`@world`, `-u`, `--undo`).

**`--install-pkgbuild <PATH>`** собирает и ставит локальный каталог с PKGBUILD через тот же конвейер, что AUR/ABS (сканер → песочница), а не голым `makepkg -si`. В `PATH` должен лежать `PKGBUILD`; отсутствующий `.SRCINFO` создаётся. Сочетается с `--pkgbuild-view`, `-a`, `-1`, `--skippgp`, `--no-sandbox`, `--unshare-net-build` и `--skip-srcinfo-regen`, но не с пакетами/`@наборами` и `--aur`/`--abs`. В `world` пакет записывается как `Err/` (источник неизвестен), если не указан `-1`.

**`--scan`** проверяет PKGBUILD (+ `.install`), ничего не собирая и не ставя: для пакета из AUR берёт файлы из cgit, для локального каталога - `--install-pkgbuild <PATH> --scan`. Отчёт тот же, что при обычном сканировании перед сборкой, только без вопроса «Continue anyway?». Завершается с ненулевым кодом при любой находке, так что годится для скриптов. Для `--abs` пока недоступен.

```bash
emerge neovim-git --aur --pkgbuild-view
emerge --install-pkgbuild ~/src/my-pkg --pkgbuild-view --skippgp
emerge neovim-git --aur --scan
```

### Devel-пакеты (`-git`, `-hg`, `-svn`, `-bzr`)

Версия на странице AUR меняется только когда сопровождающий перезаливает пакет, поэтому обычный `-u` часто не замечает изменений в апстриме devel-пакетов.

`-u --devel` (обновление) и `--check-devel` (только отчёт) закрывают этот пробел: для каждого установленного devel-пакета берут URL `git+` из PKGBUILD, делают `git ls-remote` (клон не нужен) и сравнивают с хэшем из прошлого раза (`~/.cache/aura-emerge/devel.state`). Пока только git; `-hg`/`-svn`/`-bzr` выводятся как «couldn't determine», а не пропускаются молча.

```bash
emerge -u --devel        # обновить + пересобрать devel-пакеты, чей апстрим ушёл вперёд
emerge --check-devel     # только отчёт
```

### Кэш исходников

`SRCDEST` по умолчанию - `~/.cache/aura-emerge/sources` (или `$XDG_CACHE_HOME/aura-emerge/sources`) для **всех** сборок AUR/ABS, не только VCS: makepkg повторно использует скачанные tarball'ы и делает инкрементальный fetch. Чекауты PKGBUILD AUR/ABS живут в `~/.cache/aura-emerge/build/{aur,abs}/` и обновляются на месте (`git fetch` / `git pull`), а не стираются каждый запуск. Если `SRCDEST` задан в `makepkg.conf`, используется он. Очистить кэш исходников: `emerge --clean-source-cache`.

### `@preserved-rebuild` и `--revdep-rebuild`

В Arch нет учёта preserved-libs. Пробел закрывают два инструмента:

- **`@preserved-rebuild`** собирает *объявленные* зависимости всех установленных пакетов (`pacman -Qi`), проверяет их через `pacman -T` и предлагает переустановить недостающие с `--asdeps` (чтобы их не затянуло в world последующим `--regen-world-from-explicit`).
- **`--revdep-rebuild`** читает установленные ELF-файлы (`DT_NEEDED`, `DT_RPATH`/`DT_RUNPATH`) и находит бинарники, которые линкуются с уже не существующим `.so` - классическая поломка после обновления `openssl`/`icu`/`boost`. Пакеты AUR/локальные пересобираются через песочницу; пакеты официальных репозиториев здесь пересобрать нельзя, поэтому недостающая библиотека ищется через `pacman -F`, а пакет, который её даёт, предлагается к установке. `--exclude` и [маска](#packagemask) действуют.

```bash
emerge @preserved-rebuild -p     # только список
emerge --revdep-rebuild          # просканировать, затем спросить перед исправлением
emerge --revdep-rebuild -p       # только скан
```

### `world`

`/etc/portage/world` - список всех явно установленных пакетов. Записи добавляются при установке, убираются при удалении и правятся вручную (один пакет на строку).

```
core/nano
extra/firefox
aur/ayugram-desktop-bin
abs/nano
chaotic-aur/visual-studio-code-bin
```

**Префиксы:** имя официального репозитория, `aur/`, `abs/`, `Err/` (источник неизвестен).

> **Об `Err/`:** используется для локальной сборки без прослеживаемого происхождения и для записи `abs/`/`aur/`, которая перестала разрешаться. `--regen-world` и `--regen-sets` сохраняют известный префикс `abs/`/`aur/`, а не откатываются к `Err/`.

Если в пакетной установке одного имени нет, остальные всё равно ставятся; отсутствующие выводятся в конце с ненулевым кодом выхода.

**У `@world` два значения:**

- `emerge @world` - *подготовка*: ставит то, что есть в `world`, но ещё не установлено. Держите `world` в dotfiles, клонируйте на новой машине, запускайте `emerge @world`.
- `emerge -u` (или `emerge -u @world`) - *обновление*: обновляет всё установленное (репозитории + AUR). `world` не читает.
- `emerge --sync` - только обновление баз (`-Sy` / `-Syy`).

### Пользовательские наборы

`/etc/portage/sets/<name>` (или `<name>.set`) вызывается как `@<name>`. Один пакет на строку, комментарии `#` допускаются. Строки вида `@other` вкладывают другой набор (раскрывается рекурсивно, циклы безопасны). Установка `@name` записывает `@name` в world, поэтому члены набора защищены от depclean, пока их не уберут из файла набора.

```
# /etc/portage/sets/game-kit
steam
lutris
gamemode
aur/protonup-qt
```

```bash
emerge @game-kit @dev-tools neovim    # наборы свободно комбинируются
emerge --list-sets                    # список наборов (также: emerge @<TAB>)
emerge -s @game-kit                   # откуда берётся каждый пакет: репо / aur / not found
```

Если есть и `game-kit`, и `game-kit.set`, `@game-kit` отклоняется как неоднозначный. `@world` и `@preserved-rebuild` - это действия, поэтому `-s @world` и `-s @preserved-rebuild` дают ошибку.

### `--undo`

Откатывает последнюю успешную установку, удаление или depclean (один шаг истории):

- **после установки** - удаляет только пакеты, которые в этом запуске были новыми, и возвращает установленные пакеты, удалённые как блокеры;
- **после удаления** - переустанавливает из записанного источника (`Err/`/`abs/` только перечисляются);
- **после depclean** - переустанавливает удалённых сирот **как зависимости** (в world не добавляются): пакеты из репозиториев - из репозиториев, локальные сборки - через AUR, если он их ещё содержит. Версии записываются: если в репозиториях теперь другая, вам об этом скажут; пакет, которого больше нигде нет, перечисляется со строкой `pacman -U` для файла из кэша. Если часть пакетов не встала, в сохранённом состоянии остаются только они, и `--undo` можно запустить снова;
- **после `-u`** - не поддерживается (нет снимка версий).

Состояние хранится в `/etc/portage/lastaction.state` (`--undo`) и `/etc/portage/resume.state` (`--resume`).

### Новости

`emerge --news` получает [RSS-ленту новостей Arch Linux](https://archlinux.org/feeds/news/). Состояние прочитанного хранится в `~/.cache/aura-emerge/news.state` (root не нужен).

```bash
emerge --news        # список, непрочитанные помечены [N]
emerge --news 3      # прочитать запись 3
emerge --news all    # пометить всё прочитанным
```

## Безопасность

### Сканирование PKGBUILD

Перед каждой установкой из AUR исходный `PKGBUILD` (и любой хук `.install`) загружается из cgit и сканируется в два слоя:

1. **AST** (`tree-sitter-bash`): `curl|sh`, `base64`/`xxd | sh`, `source <(curl …)`, `eval "$(curl …)"`, `sudo`/`pkexec`/`doas`, inline-выполнение для python/perl/ruby/node/lua, `sh -c` с подозрительной нагрузкой, подстановка команд на верхнем уровне, обходы через конкатенацию строк и т. д.
2. **Построчные эвристики:** `chmod 777`, «голые» IPv4, сторонние менеджеры пакетов (`npm`/`pnpm`/`bun`/`yarn`/`pip`/`gem install …`, только удалённые `cargo install`/`go install`), `bash <(curl …)`, `curl … | python3/perl/node/…`, распаковка сразу в шелл (`gunzip`/`zstd -d`/`unzip -p`/`tar -xO | sh`), каналы `/dev/tcp`//dev/udp`, heredoc'и, скармливаемые шеллу (или с fetch/decode внутри тела), paste-сайты, `.onion`, проверки на антипесочницу, известные IOC кампаний (Atomic Arch, openconnect-sso, `compromised.txt` 2018 года, …).

Находки бывают `Suspicious` или `ConfirmedIoc`. Срабатывание не блокирует жёстко: вы получаете файл/строку, ссылку на cgit и `Continue anyway? [y/N]`.

**Вспомогательные файлы:** когда checkout лежит на диске (сборка или `--install-pkgbuild <PATH> --scan`), `Makefile` (`Makefile`/`makefile`/`GNUmakefile`) и shell-скрипты, на которые ссылается PKGBUILD (`*.sh`, `*.bash`, `*.zsh`, `*.fish`), тоже проверяются построчно: `curl|sh`, base64/hex в shell, «голые» IP и остальные shell-IOC. В находке указаны файл и строка. `--scan <pkg> --aur` видит только то, что отдаёт cgit (`PKGBUILD` и `.install`). Патчи пока не сканируются.

### Песочница

Сборки **AUR и ABS** выполняют `pkgver()` / `prepare()` / `build()` / `check()` / `package()` внутри [`bwrap`](https://github.com/containers/bubblewrap). Это обязательно; отказаться можно только явным `--no-sandbox`.

- пустой tmpfs поверх `/home` и `/run` (нет настоящего `$HOME`, нет сессионной D-Bus / ssh-agent / gpg-agent);
- `--clearenv` плюс небольшой белый список (`PATH`, `TERM`, `LANG`, каталоги назначения сборки, …);
- копия связки ключей GnuPG только с публичными ключами;
- PKGBUILD и `*.install` повторно монтируются только для чтения;
- запись только в каталог сборки (+ настроенные `PKGDEST` / `SRCDEST` / …);
- сеть по умолчанию общая; остальные пространства имён отключены;
- `fakeroot` в `package()` работает во вложенном `bwrap` с uid 0.

| Флаг | Эффект |
|------|--------|
| `--no-sandbox` | Отключить bwrap; обычный `makepkg -si` (также пропускает подтверждение безаудитного fallback'а) |
| `--unshare-net-build` | Сеть только для загрузки/`prepare()`; `build()`/`check()`/`package()` - без сети |

Если `bubblewrap` не установлен, сборка отклоняется.

**Третий барьер перед установкой:** готовый `*.pkg.tar.*` проверяется через `bsdtar` на setuid/setgid, скрипты `.INSTALL` и чувствительные пути (хуки alpm, юниты systemd, sudoers, udev, tmpfiles.d, binfmt.d, pam.d, dbus system.d, dispatcher.d у NetworkManager, environment.d, sysusers.d, …). `.INSTALL` повторно сканируется сканером PKGBUILD. С `-a` нужно подтвердить; с `--noconfirm` - предупреждение и продолжение; упавший аудит bsdtar останавливает установку вместо безаудитной.

**Единственный подтверждаемый fallback:** PKGBUILD, чьи зависимости не удаётся разрезолвить статически (нет `.SRCINFO` и динамический `depends=(…)`), - единственный путь сборки, куда пакет может завести сам автор и который заканчивается голым `makepkg -si` без песочницы и аудита. Молча он не запускается: в интерактиве - явный вопрос Да/Нет для этого пакета; в `--noconfirm`-запуске - отказ (fail-closed). `--no-sandbox` пропускает этот вопрос - это осознанный отказ от песочницы на весь запуск.

Установка зависимостей (`pacman -S`) и финальный `pacman -U` выполняются **вне** песочницы - они не запускают код PKGBUILD.

### Привилегии

Фронтенд никогда не работает от root. Если запуск от root (`sudo emerge …`, `doas`, `pkexec`, `su -`), он сначала стартует root-helper, затем навсегда понижает права до вызвавшего пользователя (берётся из `AE_USER`, `SUDO_UID`, `DOAS_USER`, `PKEXEC_UID` или audit login uid; если ничего не найдено - остановка). При запуске от обычного пользователя helper стартует через `sudo` при первой необходимости. Резолвер, сборки AUR/ABS, песочница и все кэши работают без привилегий; работа через libalpm (install/remove/sync/upgrade/`-U`, проверка подписей) идёт через helper (`--ae-service`), который проверяет каждый запрос и сам применяет `package.mask` (`helper refused: … masked by …`).

### Прочее усиление

- Текст новостей Arch очищается (нельзя внедрить ANSI/bidi в терминал или `news.state`).
- Файлы конфига, маски, env и world открываются с `O_NOFOLLOW`, поэтому символьную ссылку нельзя подсунуть через гонку.
- Rust-зависимости проверяются через [`cargo deny`](https://github.com/EmbarkStudios/cargo-deny) (`make deny`).

<details>
<summary><h3>Проверка сканера (после установки)</h3></summary>

```bash
git clone https://github.com/Undercat037/aura-emerge.git
cd aura-emerge
emerge --install-pkgbuild ./scaner-test --ask
```
</details>

<details>
<summary><h2>Флаги</h2></summary>

| Флаг | Описание |
|------|------|
| `-s`, `--search` | Поиск пакетов |
| `-S`, `--searchdesc` | Поиск по описанию |
| `--search-all` | С `-s`: показать одно и то же имя из каждого репозитория (как `pacman -Ss`) |
| `-sv` | Подробная информация о пакете |
| `-v`, `--verbose` | Подробный вывод / подробная информация при поиске (`-sv`) |
| `-u`, `--update` | Полное обновление системы (world не читает) |
| `--devel` | С `-u`: заодно пересобрать `-git`/`-hg`/`-svn`/`-bzr`, чей апстрим ушёл вперёд |
| `--check-devel` | Показать devel-пакеты с изменениями в апстриме (без пересборки) |
| `-C`, `--unmerge` | Удалить пакеты (unmerge) |
| `-c`, `--depclean` | Удалить сирот (depclean) |
| `--show-protected` | С `-c`: показать сирот, которые остались только потому, что есть в world |
| `-p`, `--pretend` | Pretend (пробный прогон) |
| `-a`, `--ask` | Спросить перед действием |
| `--sudoloop` | Держать метку времени sudo «тёплой» (обновление каждые 60 с) весь запуск |
| `-1`, `--oneshot` | Установить как зависимость (в world не попадает) |
| `-n`, `--noreplace` | Не переустанавливать, если уже есть (та же или более новая версия) |
| `--with-optdeps` | Заодно ставить опциональные зависимости пакетов из официальных репозиториев |
| `--aur` | Принудительно только AUR (зависимости только из AUR всегда собираются рекурсивно) |
| `--repos` | Только официальные репозитории (AUR не трогать) |
| `--abs` | Сборка из ABS (`pkgctl`) в песочнице bwrap |
| `--no-sandbox` | Отключить bwrap для AUR/ABS; обычный makepkg без изоляции (также пропускает подтверждение fallback'а) |
| `--unshare-net-build` | Без сети во время build()/check()/package() (в `prepare()` сеть остаётся) |
| `--skippgp` | Пропустить проверки PGP при сборках ABS (`--skippgpcheck`) |
| `--autopgp` | Автоимпорт недостающих PGP-ключей перед сборкой `--abs` |
| `--edit` | Править PKGBUILD в `$EDITOR` перед сборкой (только для запрошенного пакета); после сохранения перегенерирует `.SRCINFO` |
| `--skip-srcinfo-regen` | С `--edit`: не перегенерировать `.SRCINFO` |
| `--pkgbuild-view` | Показать/сравнить PKGBUILD и подтвердить перед сборкой (только для запрошенного пакета; при отказе предлагает `$EDITOR`) |
| `--install-pkgbuild <PATH>` | Собрать и установить локальный каталог с PKGBUILD (сканер + bwrap); в world записывается как `Err/`, если нет `-1` |
| `--scan` | Только аудит PKGBUILD/.install, без сборки/установки; ненулевой код при находках (AUR или `--install-pkgbuild`) |
| `--sync` | Синхронизировать базы пакетов |
| `--refresh` | Принудительный `-Syy` (с `--sync` или `-u`) |
| `--info` | Информация о системе, как `emerge --info` в Gentoo |
| `--news [N\|all]` | Новости Arch Linux: без аргумента - список, `N` - прочитать запись, `all` - пометить всё прочитанным |
| `--check-news [N\|all]` | Псевдоним `--news` |
| `--select` | Добавить в world без установки |
| `--deselect` | Убрать из world без удаления |
| `--check-world` | Показать записи world, которых нет в системе или которые установлены не явно |
| `--sync-explicit` | Пометить как зависимости явно установленные пакеты, которых нет в world, в `@set` из world и в `package.profile` (чтобы `-c` их видел). Отдельно: выполняется сразу (`-p` показывает список, `-a` спрашивает). С установкой/обновлением: после успешного завершения. Можно в `EMERGE_DEFAULT_OPTS` |
| `--prune` | Удалить пакеты, которых нет в world |
| `--regen` | Пересоздать кэш метаданных пакетов |
| `--regen-world` | Заново определить префиксы репозиториев в world |
| `--regen-sets <SET>` | Заново определить префиксы в пользовательском наборе (`@game-kit` или `game-kit`) |
| `--regen-sort` | С `--regen-sets`: заодно отсортировать файл (комментарии и группировка пропадают) |
| `--regen-world-from-explicit` | Миграция: добавить в world явно установленные пакеты (`pacman -Qeq`), которых нет в world, в `@set` из world и в `package.profile` (разово). `-p` только показывает, `-a` спрашивает |
| `--err-install` | С `@world`: заодно ставить записи `Err/` (источник неизвестен) |
| `--resume` | Продолжить прерванную транзакцию |
| `--skipfirst` | С `--resume`: пропустить первый пакет |
| `--undo` | Отменить последнюю установку/удаление/depclean (один шаг; `-u` не откатывается) |
| `--clean-source-cache` | Очистить постоянный кэш исходников VCS и выйти |
| `--batchinstall <FILE>` | Массовая установка из текстового списка (формат как у пользовательского набора) |
| `--list-sets` | Список доступных `@наборов` (используется и автодополнением) |
| `--exclude <ATOM>` | Исключить пакет из этого запуска (повторяемый/через запятую) |
| `--keep-going` | Не останавливать пакетную операцию на первой ошибке; сбои - в сводке в конце |
| `--revdep-rebuild` | Пересобрать пакеты, чьи бинарники линкуются с отсутствующей библиотекой |
| `-t`, `--tree` | Показать план как дерево зависимостей |
| `-D`, `--deep[=N]` | С `-t`: вложенность на N уровней, без значения - на все |
| `--jobsr <N>` | Макс. параллельных установок из официальных репозиториев (по умолчанию 1) |
| `--jobsa <N>` | Макс. параллельных сборок AUR/ABS (по умолчанию 1) |
| `--jobs <N>` | Псевдоним `--jobsr` (в стиле Portage) |
| `--load-average <X>` | Не запускать новые сборки, пока средняя нагрузка за минуту выше X |
| `--nospinner` | Без живой строки Jobs |
| `--color <y\|n\|auto>` | Принудительно включить или выключить цвет |
| `--quiet-build=n` | То же, что `--debug` (написание Gentoo) |
| `--list-profiles` | Список профилей в стиле Portage (как `eselect profile list`) |
| `--set-profile <N\|PATH>` | Задать `/etc/portage/make.profile`: индекс из `--list-profiles` или путь |
| `--aeforce` | AE Force: переносить файлы, мешающие установке, в `<file>.aeforce` без вопроса |
| `--noaefile` | AE Force: не оставлять файлы `.ae*` (старый сохранить, новый отбросить; с `--aeforce`: отбросить старый) |
| `--aelist` | Список файлов, оставленных AE Force (`.aenew` / `.aeforce` / `.aepark`) |
| `--aeclean [new\|old\|park\|all]` | Удалить файлы AE Force после вопроса (без аргумента = `all`) |
| `--ignore-default-opts` | Игнорировать `EMERGE_DEFAULT_OPTS` из make.conf на этот запуск |
| `--debug` | Показывать вывод makepkg/компилятора вживую (также `AE_DEBUG=1`); по умолчанию он скрыт ради читаемости строк `>>>`, а при падении сборки сохранённый лог всё равно выводится |
| `--log <PATH>` | Записать лог сессии (команда, строки emerge.log, полный вывод сборки) для баг-репортов - `| tee` не нужен; только дозапись: существующее содержимое не обрезается и не перезаписывается |
| `-N`/`--newuse`, `-e`/`--emptytree`, `-q`, `-O`, `-o`, `-k`, `-K`, `-g`, `-G`, `-b`, `-B`, `-f`, `-F`, `-l`, `-A`, `--nodeps`, `--onlydeps`, `--complete-graph`, `--changed-use`, `--backtrack`, `--columns`, `--newrepo`, `--reinstall`, `--with-bdeps`, `--metadata`, `--clean`, `--config`, `--noconfmem`, `--verbose-conflicts` | Совместимость с Gentoo: принимаются, ничего не делают |
| `-h`, `--help` | Справка |
| `-V`, `--version` | Версия |

Страница руководства `emerge(1)` генерируется из CLI (`emerge --gen-manpage`), поэтому не может разойтись с `--help`.
</details>

## Автор / Лицензия
Copyright (C) 2026 **Undercat037**
Лицензия GPL-3.0
