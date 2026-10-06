<div align="center">
  <img src="misc/logo/slash-logo-128x128.svg" width="128" alt="aura-emerge logo">

# Aura-Emerge Secure Package Manager

[![AE_Site](https://img.shields.io/badge/AE_Site-333333?style=flat-square&logo=bookstack&logoColor=FFFFFF)](https://undercat037.github.io/aura-emerge/)
[![AUR](https://img.shields.io/badge/AUR-333333?style=flat-square&logo=arch-linux&logoColor=1793D1)](https://aur.archlinux.org/packages/aura-emerge)
[![GitHub](https://img.shields.io/badge/GitHub-333333?style=flat-square&logo=github&logoColor=FFFFFF)](https://github.com/Undercat037/aura-emerge)
[![GitLab](https://img.shields.io/badge/GitLab-333333?style=flat-square&logo=gitlab&logoColor=FC6D26)](https://gitlab.com/Undercat037/aura-emerge)

[en](README.MD) | [ua](UA-README.MD) | ru
</div>

> Автономный менеджер пакетов `emerge` в стиле Gentoo для Arch Linux: ставит пакеты из официальных репозиториев, AUR и ABS; перед сборкой сканирует PKGBUILD на признаки атак на цепочку поставок; ненадёжные шаги сборки выполняет внутри песочницы `bwrap`.

Пакеты учитываются в файле `world` - установил один раз, отслеживается всегда.

## Установка

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
<summary><h3>Вручную/makepkg (нестабильно)</h3></summary>

### Make
```bash
git clone https://github.com/Undercat037/aura-emerge.git
cd aura-emerge
make install
```
### Ручная сборка
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

**Зависимости:** `git`, `bubblewrap` (песочница сборки для AUR/ABS)  
**Опционально:**
- `devtools` - поддержка `--abs` (`pkgctl repo clone`)
- `gnupg` - автоматический импорт PGP-ключей (`--autopgp`)

## Gentoo → Arch: аналог make.conf

В Arch нет USE-флагов, но компиляцию можно настроить так же, как в `/etc/portage/make.conf` у [Gentoo](https://wiki.gentoo.org/wiki/Main_Page), - через `/etc/makepkg.conf`:

```bash
sudo nvim /etc/makepkg.conf
```

| Gentoo (`make.conf`)         | Arch (`makepkg.conf`)         | Назначение                       |
|------------------------------|-------------------------------|----------------------------------|
| `CFLAGS="-O2 -march=native"` | `CFLAGS="-O2 -march=native"`  | Флаги оптимизации компилятора    |
| `CXXFLAGS="-O2 -march=..."`  | `CXXFLAGS="-O2 -march=..."`   | Флаги компилятора C++            |
| `RUSTFLAGS="-C opt-level=3"` | `RUSTFLAGS="-C opt-level=3"`  | Флаги компилятора Rust           |
| `MAKEOPTS="-j$(nproc)"`      | `MAKEFLAGS="-j$(nproc)"`      | Параллельные задачи сборки       |
| `USE="lto"`                  | `OPTIONS+=(lto)`              | Оптимизация на этапе компоновки  |
| `FEATURES="ccache"`          | `BUILDENV+=(ccache)`          | Кэш сборки                       |
| `PORTDIR` / `PKGDIR`         | `PKGDEST`, `SRCDEST`          | Каталоги артефактов сборки       |

См. [флаги оптимизации GCC](https://gcc.gnu.org/onlinedocs/gcc/Optimize-Options.html) и [параметры кодогенерации rustc](https://doc.rust-lang.org/rustc/codegen-options/index.html).

[**→ Пример makepkg.conf**](misc/makepkg.conf)

## make.conf

`/etc/portage/make.conf` — собственный конфигурационный файл aura-emerge (только system path), отдельный от `makepkg.conf`: как в Gentoo, где `EMERGE_DEFAULT_OPTS` и `CFLAGS` живут в одном `make.conf`.

```bash
sudo nvim /etc/portage/make.conf
```

```bash
# Флаги, подставляемые в каждый запуск перед тем, что набрано в
# командной строке (при конфликте командная строка всегда главнее).
# Отключить для одного запуска: --ignore-default-opts.
EMERGE_DEFAULT_OPTS="--pkgbuild-view --unshare-net-build"

# Флаги сборки, применяются через сгенерированный makepkg.conf
# (makepkg --config) - переопределяют /etc/makepkg.conf, не трогая его.
CFLAGS="-march=native -O2 -pipe"
CXXFLAGS="$CFLAGS"
RUSTFLAGS="-C target-cpu=native"
MAKEFLAGS="-j$(nproc)"
NINJAFLAGS="-j$(nproc)"
OPTIONS=(strip docs !debug lto)
BUILDENV=(!distcc color check !sign)

# Переключатели в стиле Portage, переводятся в makepkg.
FEATURES="ccache -test"
```

Распознаваемые ключи: `EMERGE_DEFAULT_OPTS`, `FEATURES`, `CFLAGS`, `CXXFLAGS`, `CPPFLAGS`, `LDFLAGS`, `RUSTFLAGS`, `MAKEFLAGS`, `NINJAFLAGS`, `BUILDENV`, `OPTIONS`. Синтаксис плоский, как присваивание в bash: `KEY="..."` или `KEY=(a b c)`.

Токены `FEATURES` (`-name` выключает токен), у которых есть аналог в makepkg, при сборке вносятся в системные массивы; запись, которую вы сами указали в `BUILDENV`/`OPTIONS`, главнее:

| `FEATURES`   | makepkg                  |
|--------------|--------------------------|
| `ccache`     | `BUILDENV` `ccache`      |
| `distcc`     | `BUILDENV` `distcc`      |
| `test`       | `BUILDENV` `check`       |
| `nostrip`    | `OPTIONS` `!strip`       |
| `splitdebug` | `OPTIONS` `debug`        |

Скаляры и списки взаимозаменяемы везде: список склеивается через пробел, строка разбивается по пробелам. Значения раскрываются оболочкой, когда makepkg читает сгенерированный конфиг, поэтому `-j$(nproc)` работает, как и ссылка на другой ключ, заданный выше, например `CXXFLAGS="$CFLAGS"`; значение в одинарных кавычках (`'\$FOO'`) оставляет знак доллара как есть.

`EMERGE_DEFAULT_OPTS` принимает только флаги-модификаторы (белый список: `--ask`, `--verbose`, `--devel`, `--tree`, `--searchdesc`, `--exclude=<atom>` и подобные - см. [файл-пример](misc/make.conf)), так что превратить каждый запуск в `--unmerge` нельзя. Конфликтующие пары (`--aur`/`--abs`, `--aur`/`--repos`, `--no-sandbox`/`--unshare-net-build`, `--skippgp`/`--autopgp`, `--select`/`--deselect`) отклоняются, если заданы обе, неважно - набраны или пришли из конфига; значение из конфига, конфликтующее с набранным в командной строке, отбрасывается, поскольку командная строка выражает более конкретное намерение. `--abs` и `--repos` **не** конфликтуют: ABS собирает пакет официального репозитория из его же исходников, так что они согласуются.

`--info` показывает, что было прочитано, и итоговое значение каждого переопределения после раскрытия оболочкой.

[**→ Пример make.conf**](misc/make.conf)

## Маска (/etc/portage/package.mask)

`package.mask` из Portage, для атомов Arch: пакеты, которые эта машина никогда не ставит; проверяется даже для транзитивной зависимости из AUR и даже при запросе по имени. Одна запись на строку, допускается глоб `*`, необязательный префикс репозитория, необязательный завершающий `# комментарий`. `/etc/portage/package.mask` может быть одним файлом или каталогом: в каталоге читается каждый обычный файл (любое имя, расширение не нужно, файлы с точки пропускаются) в порядке имён. Символьная ссылка на конечный файл отклоняется (открытие с `O_NOFOLLOW`).

```
ayugram-desktop-bin            # конкретный пакет, из любого источника
aur/*-bin                      # никогда не ставить -bin пакеты из AUR
*-git                          # на этой машине без devel-пакетов
extra/nano                     # конкретный пакет из конкретного репозитория
```

Замаскированный пакет жёстко останавливает запрос - без вопроса и без флага для обхода. Это отдельно от `--exclude` ниже: маска - постоянное решение, а `--exclude` действует на один запуск.

## package.env (/etc/portage/package.env)

`package.env` из Portage: флаги сборки для отдельных пакетов. Каждая запись - атом и одно или несколько имён env-файлов из `/etc/portage/env/`; эти файлы накладываются поверх [make.conf](#makeconf) только на сборку этого пакета. `/etc/portage/package.env` может быть одним файлом или каталогом файлов (любое имя, файлы с точки пропускаются, чтение в порядке имён), как и `package.mask`.

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

Наложение: `make.conf`, затем каждая подходящая запись в порядке файлов и строк, затем каждый env-файл в том порядке, в котором он назван. Побеждает слой, задавший ключ последним, причём ключ заменяется целиком, а не сливается: `OPTIONS` в env-файле заменяет `OPTIONS` из `make.conf`, поэтому перечисляйте все нужные опции. Если два слоя задают один ключ разными значениями, сборка напишет, какое победило. Пакет подходит по своему pkgbase или любому из pkgname.

Затрагиваются только сборки AUR/ABS (официальные пакеты здесь не собираются). `EMERGE_DEFAULT_OPTS` в env-файле игнорируется с предупреждением. Файл-символьная ссылка, некорректная запись или отсутствующий env-файл пропускаются с предупреждением, где указаны файл и строка; остальное применяется. Сборка выводит использованные файлы: `>>> applying build flags from /etc/portage/make.conf, /etc/portage/env/lto.conf`.

## --exclude / --keep-going

`--exclude <atom>` (повторяемый или через запятую) исключает пакет из текущего запуска - как `--exclude` в Portage. Действует на установки, `-u` (через `pacman --ignore`), подготовку `@world`, `--depclean`/`--prune` (защищает от удаления) и `--revdep-rebuild`.

```bash
emerge -u --exclude linux --exclude aur/ayugram-desktop-bin   # обновить всё, кроме этих
emerge --exclude firefox @world                               # подготовка, пропустить firefox, даже если его нет
```

`--keep-going` не останавливает пакетную операцию (`@world`, `-u`, `--batchinstall`, установку нескольких пакетов) на первой ошибке: она продолжается, запоминает сбои и в конце печатает сводку с ненулевым кодом выхода. `--resume` после запуска с `--keep-going` повторяет только упавшие пакеты, а не весь пакет целиком.

```bash
emerge -u --keep-going                # полное обновление, не останавливаться на одном плохом пакете из AUR
emerge --batchinstall pkgs.txt --keep-going
emerge --resume                       # повторяет только то, что упало в прошлый раз
```

## -t/--tree, --deep

`-t`/`--tree` показывает план (с `-p`/`--pretend` или прямо перед реальной установкой) в виде дерева зависимостей вместо плоского списка, как `emerge -pt`. По умолчанию один уровень вложенности: набранное - на глубине 0, всё, что оно притянуло как зависимости, - с отступом под ним; принадлежность определяется сопоставлением «Depends On» из `pacman -Si` для каждого запрошенного пакета со всем остальным в плане. Добавьте `--deep`, чтобы вложенность шла на сколько уровней уходит реальная цепочка зависимостей, или `--deep=N`, чтобы ограничить её N уровнями вместо остановки на прямых зависимостях.

```bash
emerge -apt noctalia --aur     # ask + pretend + tree: просмотр перед сборкой
emerge -pt noctalia --aur --deep   # то же, но вложенность до самого низа
emerge deepin-session --ask --tree --deep=3   # установка, дерево не глубже 3 уровней
```

```
[ebuild  N ] noctalia-5.1.0-1.1
[ebuild  N ] `-- libqalculate-5.12.0-1.1
[ebuild  N ]   `-- sdbus-cpp-2.3.1-1.1
```

Зависимость, которую притянул pacman, но которая буквально не названа ни в чьём «Depends On» (разрешена через `provides` или лежит за пределами `--deep`), всё равно показывается, просто без конкретного родителя, под которым её можно вложить: это реальная информация, а не выдуманное дерево. Для чистой установки через `--aur` шаг плана пока не разрешает транзитивные зависимости, присутствующие только в AUR, поэтому `-t` там сейчас плоский, как и без него.

## Журнал merge/unmerge

`/var/log/emerge.log` - `emerge.log` из Portage, для атомов pacman/AUR/ABS. Только дозапись, одна строка на событие; пишется так же, как прочее состояние `/etc/portage/*`, принадлежащее root (`sudo tee`, по возможности - неудачная запись в журнал никогда не проваливает merge/unmerge, который уже произошёл).

```
2026-09-21 08:51:44  MERGE    aur    noctalia-5.1.0-1.1        (12s)
2026-09-21 08:52:10  MERGE    aur    libqalculate-5.12.0-1.1   (9s)
2026-09-21 09:03:02  MERGE    repo   nano-8.0-1  vim-9.1-1     (4s)
2026-09-21 09:10:05  UNMERGE  -      old-package-2.0-1
```

Длительность всегда настоящая: пакеты AUR/ABS собираются по одному в собственном цикле этого инструмента, поэтому у каждого свой таймер; транзакция `pacman -S`/`-Syu` - это один код возврата на всю пачку, так что такая строка содержит все атомы и общее время пачки, а не выдуманное разбиение. `--info` показывает накопленные итоги (число merge/unmerge, суммарное время сборки).

## Использование

```bash
emerge neovim                              # официальные репозитории, при неудаче - AUR
emerge neovim-git --aur                    # только AUR (зависимости только из AUR собираются рекурсивно)
emerge ayugram-desktop-bin --aur --sudoloop   # то же, но sudo держится «тёплым» на всю рекурсивную сборку
emerge neovim --repos                 # только официальные репозитории, AUR не трогать
emerge neovim --abs                        # сборка из ABS (pkgctl), в песочнице bwrap
emerge neovim --abs --no-sandbox           # ABS без песочницы (обычный makepkg -si)
emerge neovim --abs --skippgp              # ABS, пропустить проверку PGP-подписей
emerge neovim --abs --autopgp              # ABS, автоимпорт недостающих PGP-ключей
emerge neovim --abs --edit                 # ABS, правка PKGBUILD ($EDITOR), перегенерирует .SRCINFO
emerge neovim-git --aur --edit             # AUR, правка PKGBUILD, перегенерирует .SRCINFO
emerge neovim-git --aur --edit --skip-srcinfo-regen  # правка без перегенерации .SRCINFO
emerge neovim-git --aur --unshare-net-build     # без сети во время build()/check()/package()
emerge -c                                  # удалить сирот (depclean)
emerge -C neovim                           # удалить пакет (unmerge)
emerge @world                              # подготовка: поставить недостающее из world
emerge @world --err-install                   # заодно обычным путём повторить записи Err/ (неизвестный источник)
emerge -u @world                           # полное обновление системы (репозитории + AUR); world не читает
emerge -u @world --refresh                 # обновление с принудительным -Syy в начале
emerge --sync                              # синхронизировать базы пакетов
emerge @game-kit                           # пользовательский набор из /etc/portage/sets/game-kit[.set]
emerge @game-kit neovim                    # наборы + обычные пакеты
emerge --list-sets                         # список доступных @наборов
emerge --info                              # информация о системе (как emerge --info в Gentoo)
emerge --news                              # новости Arch Linux, непрочитанные помечаются
emerge --news 3                            # прочитать запись 3 целиком
emerge --news all                          # пометить всё прочитанным
emerge neovim-git --aur --edit --pkgbuild-view  # показать/сравнить и PKGBUILD, поверх --edit
emerge neovim --abs --pkgbuild-view        # то же для сборки из ABS
emerge --install-pkgbuild ~/src/my-pkg        # собрать+установить локальный PKGBUILD (сканер + bwrap, как AUR/ABS)
emerge --install-pkgbuild ~/src/my-pkg --pkgbuild-view --skippgp
emerge --scan neovim-git --aur              # только аудит, без сборки
emerge --install-pkgbuild ~/src/my-pkg --scan  # то же для локального каталога
emerge -u --devel                           # полное обновление + проверка апстрима для -git/-hg/-svn/-bzr
emerge --check-devel                        # только сообщить, какие devel-пакеты сдвинулись в апстриме
emerge -s neovim                           # поиск: репозитории, затем ABS, затем AUR (отдельный блок на каждый)
emerge -s neovim --repos                   # поиск только по официальным репозиториям
emerge -s abs/neovim                       # поиск только по каталогу ABS (core/extra/multilib)
emerge -s @fonts                           # откуда берётся каждый пакет набора: репо / aur / not found
emerge -S neovim                           # поиск по описанию (--searchdesc)
emerge -sv neovim                          # подробная информация о пакете
emerge -sv neovim-git --aur                # подробная информация только из AUR
emerge -1 htop                             # установить как зависимость (в world не добавляется)
emerge -n neovim                           # не переустанавливать, если уже есть (--noreplace)
emerge -p neovim                           # пробный прогон
emerge -a neovim                           # спросить перед действием
emerge --select neovim                     # добавить в world без установки
emerge --deselect neovim                   # убрать из world без удаления
emerge --prune                             # удалить пакеты, которых нет в world
emerge --regen                             # пересоздать кэш метаданных пакетов
emerge --regen-world                       # заново определить префиксы репозиториев в world
emerge --regen-sets @game-kit              # заново определить префиксы для одного пользовательского набора
emerge --regen-sets @game-kit --regen-sort # то же + отсортировать файл по алфавиту
emerge --regen-world-from-explicit         # разовая миграция из pacman -Qeq
emerge --resume                            # продолжить прерванную транзакцию
emerge --resume --skipfirst                # продолжить, пропустив первый пакет
emerge --undo                              # отменить последнюю установку/удаление
emerge --clean-source-cache                # очистить постоянный кэш исходников VCS (-git/-hg/-svn)
emerge @preserved-rebuild                  # проверить установленные пакеты на недостающие зависимости
emerge @preserved-rebuild -p               # pretend: только список
emerge @preserved-rebuild -a               # спрашивать перед каждым запросом pacman
emerge --batchinstall pkgs.txt              # массовая установка из файла со списком
emerge --batchinstall pkgs.txt --aur -a     # сочетается с любым другим флагом
emerge -u --exclude linux                   # обновить, удержав один пакет
emerge -u --keep-going                      # обновить, не останавливаясь на первой ошибке
emerge --revdep-rebuild                     # найти и починить битые ссылки на библиотеки
emerge --revdep-rebuild -p                  # то же, только список
```

## Песочница

Сборки **AUR и ABS** выполняют `pkgver()` / `prepare()` / `build()` / `check()` / `package()` из PKGBUILD внутри [`bwrap`](https://github.com/containers/bubblewrap) (обязательно; отказаться можно через `--no-sandbox`):

- пустой tmpfs поверх `/home` и `/run` (нет настоящего `$HOME`, нет сессионной D-Bus / ssh-agent / gpg-agent);
- `--clearenv` плюс небольшой белый список (`PATH`, `TERM`, `LANG`, каталоги назначения сборки, …);
- копия связки ключей GnuPG только с публичными ключами (без приватных);
- PKGBUILD и `*.install` повторно монтируются только для чтения после монтирования каталога сборки на запись;
- запись только в каталог сборки (+ настроенные `PKGDEST` / `SRCDEST` / …);
- сеть по умолчанию общая; остальные пространства имён отключены;
- `fakeroot` в `package()` использует вложенный `bwrap` с uid 0; шим лежит в `/var/tmp` (не в `/run` и не в каталоге сборки, доступном на запись).

**Третий барьер перед установкой:** готовый `*.pkg.tar.*` проверяется (`bsdtar`) на setuid/setgid, скрипты `.INSTALL` и чувствительные пути (хуки alpm, юниты systemd, sudoers, udev, …). Содержимое `.INSTALL` повторно сканируется тем же сканером PKGBUILD. С `-a`/`--ask` нужно подтвердить; с `--noconfirm` выдаётся предупреждение и работа продолжается.

Установка зависимостей (`pacman -S`) и финальный `pacman -U` остаются **вне** песочницы - они не выполняют код PKGBUILD.

| Флаг | Эффект |
|------|--------|
| `--no-sandbox` | Отключить bwrap; обычный `makepkg -si` |
| `--unshare-net-build` | Загрузка/`prepare()` с сетью; `build()`/`check()`/`package()` выполняются без сети |

`bubblewrap` - жёсткая зависимость. Если его нет, сборка отклоняется; только явный `--no-sandbox` запускает `makepkg` без изоляции.

## Кэш исходников (SRCDEST)

Для пакетов `-git` / `-hg` / `-svn` aura-emerge по умолчанию использует постоянный `SRCDEST` в `~/.cache/aura-emerge/sources` (или `$XDG_CACHE_HOME/aura-emerge/sources`). Тогда makepkg делает инкрементальный `git fetch` вместо полного повторного клонирования при каждом запуске.

```bash
emerge --clean-source-cache   # удалить этот кэш
```

Если `makepkg.conf` уже задаёт `SRCDEST`, используется это значение.

## Пакетная установка

`--batchinstall <FILE>` читает текстовый список пакетов и вливает его в обычный список пакетов до начала работы: тот же формат, что у [пользовательского набора](#пользовательские-наборы) (один атом на строку, допускаются комментарии `#`), но из любого указанного вами пути, без предварительной регистрации в `/etc/portage/sets/`.

```
# pkgs.txt
neovim
aur/visual-studio-code-bin
firefox
```

```bash
emerge --batchinstall pkgs.txt
```

Поскольку список вливается рядом с `cli.packages`, он сочетается со всеми остальными флагами точно так же, как набранные вручную имена (`-p`, `-a`, `--aur`, `--abs`, `--repos`, ...).

## Просмотр PKGBUILD

`--pkgbuild-view` показывает PKGBUILD, который вот-вот будет собран, прямо перед началом сборки и просит подтверждения: diff с последней показанной копией этого pkgbase, если она закэширована в `~/.cache/aura-emerge/pkgbuild-view/`, а в первый раз - весь файл. При отказе предлагается открыть его в `$EDITOR` прямо там, а не просто пропустить пакет. Сочетается с `--edit` (тот выполняется первым, так что просмотр/diff отражают только что сделанные правки). Применяется к установкам через `--aur`/обычному откату на AUR и к `--abs`, всегда только для напрямую запрошенного пакета - никогда для рекурсивно притянутой зависимости из AUR и никогда для пакетных путей (подготовка `@world`, `-u`, `--undo`).

```bash
emerge neovim-git --aur --pkgbuild-view
emerge neovim --abs --edit --pkgbuild-view   # сначала правка, затем просмотр/diff результата
```

`--install-pkgbuild <PATH>` собирает и устанавливает произвольный локальный каталог с PKGBUILD через тот же конвейер, что и установка из AUR/ABS - сканер, затем песочница bwrap, - вместо голого непроверенного `makepkg -si`. В `PATH` должен лежать `PKGBUILD`; если `.SRCINFO` ещё нет, он генерируется (`makepkg --printsrcinfo`, как делает `--edit`), чтобы зависимости разрешались статически. Сочетается с `--pkgbuild-view`, `-a`/`--ask`, `-1`/`--oneshot`, `--skippgp`, `--no-sandbox`, `--unshare-net-build`, `--skip-srcinfo-regen`; несовместим с любым аргументом-пакетом/`@набором` и с `--aur`/`--abs`. Записывается в `world` с префиксом `Err/` (источник неизвестен - см. [world](#world)), если не указан также `-1`/`--oneshot`.

```bash
emerge --install-pkgbuild ~/src/my-pkg
emerge --install-pkgbuild ~/src/my-pkg --pkgbuild-view --skippgp
```

`--scan` проверяет PKGBUILD (+ `.install`), ничего не собирая и не устанавливая: для имени пакета AUR берёт файлы прямо из cgit, для локального каталога читает его напрямую через `--install-pkgbuild <PATH> --scan`. Те же находки/отчёт, что и при обычном сканировании перед сборкой, только без вопроса «Continue anyway?» (продолжать некуда). Завершается с ненулевым кодом при любой находке, так что годится для скриптов. Для `--abs` пока не подключён.

```bash
emerge --scan neovim-git --aur
emerge --install-pkgbuild ~/src/my-pkg --scan
```

## Проверка апстрима devel-пакетов

В соглашении Arch для `-git`/`-hg`/`-svn`/`-bzr` версию вычисляет `pkgver()` во время сборки по истории VCS, но версия, *записанная на самой странице AUR*, сдвигается только когда сопровождающий вручную перезаливает пакет, а для активно развиваемого проекта это может отставать от реального апстрима на недели и месяцы. Обычный `-u` сравнивает лишь эту записанную версию из AUR, поэтому такие пакеты регулярно не помечаются на пересборку, даже если апстрим давно ушёл вперёд.

`--devel` (с `-u`) и `--check-devel` (отдельно, только отчёт) закрывают этот разрыв: для каждого установленного devel-пакета они достают URL источника `git+` прямо из его PKGBUILD и выполняют по нему `git ls-remote` напрямую, без локального клона, сравнивая результат с хэшем, увиденным в прошлый раз (`~/.cache/aura-emerge/devel.state`). Пока только git; пакеты `-hg`/`-svn`/`-bzr` распознаются, но выводятся как «couldn't determine», а не молча пропускаются.

```bash
emerge -u --devel        # обычное полное обновление + пересборка devel-пакетов, чей апстрим сдвинулся
emerge --check-devel     # только сообщить, какие сдвинулись, без пересборки
```

## @preserved-rebuild

В Arch нет учёта preserved-libs. `@preserved-rebuild` собирает объявленные зависимости каждого установленного пакета (`pacman -Qi`), передаёт дедуплицированный список в `pacman -T` и предлагает переустановить неудовлетворённые через `pacman -S --asdeps` (чтобы они не выглядели «явными» и их не затянуло в world последующим `--regen-world-from-explicit`).

## --revdep-rebuild

Это не та же проверка, что `@preserved-rebuild` выше, который видит только *объявленные* зависимости через `pacman -T`. `--revdep-rebuild` читает установленные ELF-файлы напрямую (`DT_NEEDED`, `DT_RPATH`/`DT_RUNPATH`) и находит бинарники, которые линкуются с уже не существующим `.so`: классическая поломка после обновления `openssl`/`icu`/`boost`, которую Gentoo решает через preserved-libs, а Arch «из коробки» - нет.

Чужие (AUR/локальные) пакеты пересобираются обычным путём через песочницу; пакеты официальных репозиториев в бинарном дистрибутиве пересобрать нельзя, поэтому недостающие библиотеки ищутся в базе файлов (`pacman -F`), а пакеты, которые их предоставляют, предлагаются к установке. `--exclude` и [маска](#маска-etcportagepackagemask) действуют оба.

```bash
emerge --revdep-rebuild       # просканировать, затем спросить перед исправлением
emerge --revdep-rebuild -p    # только скан, показать что сломано
```

## world

Путь: `/etc/portage/world`. Каждый явно установленный пакет добавляется автоматически и убирается при удалении. Можно править вручную (один пакет на строку).

```
cachyos-extra-v3/rust-analyzer
chaotic-aur/visual-studio-code-bin
core/nano
aur/ayugram-desktop-bin
abs/nano
extra/firefox
```

Префиксы: имя официального репозитория, `aur/`, `abs/`, `Err/` (источник неизвестен).

> **Об `Err/`:** используется и для действительно локальной сборки без прослеживаемого происхождения, и для записи `abs/`/`aur/`, которая позже перестала разрешаться. `--regen-world` / `--regen-sets` сохраняют известный префикс `abs/`/`aur/`, а не сворачивают его в `Err/` при первом же промахе повторного разрешения.

Пакетные установки больше не прерывают весь запрос, если одного имени нет: остальное всё равно ставится, а отсутствующие имена выводятся в конце с ненулевым кодом выхода.

### У `@world` два значения

- **`emerge @world` (без `-u`)** - *подготовка*. Ставит всё, что указано в `world` и ещё не стоит в системе; уже установленные пакеты не трогаются. Источник определяет префикс. Держите `world` в dotfiles, клонируйте на новой машине, запускайте `emerge @world`.
- **`emerge -u @world`** (или `emerge -u`) - *обновление*. Обновляет всё уже установленное (репозитории + AUR). `world` не читает.
- **`emerge --sync`** - только обновление баз (`-Sy` / `-Syy`).

## Пользовательские наборы

`/etc/portage/sets/<name>` (или `<name>.set`) вызывается как `@<name>`:

```
# /etc/portage/sets/game-kit   (или game-kit.set)
steam
lutris
gamemode
aur/protonup-qt
```

Один пакет на строку, допускаются комментарии `#`. Если существуют и `game-kit`, и `game-kit.set`, `@game-kit` отклоняется как неоднозначный, пока один из файлов не удалён. Наборы свободно комбинируются:

```bash
emerge @game-kit @dev-tools neovim
```

`emerge --list-sets` и автодополнение оболочки (`emerge @<TAB>`) показывают доступные наборы.

`emerge -s @game-kit` ищет каждый пакет набора по точному имени и печатает, откуда он берётся (репозиторий, `aur` или `not found`). `@world` и `@preserved-rebuild` - это действия, а не цели поиска: `-s @world` и `-s @preserved-rebuild` отклоняются с ошибкой.

## --undo

Откатывает последнюю успешную установку или удаление (один шаг истории):

- **после установки** - удаляет только пакеты, которые в этом запуске были совершенно новыми;
- **после удаления** - переустанавливает из записанного источника (`Err/`/`abs/` только перечисляются);
- **после `-u`** - не поддерживается (нет снимка версий).

```bash
emerge --undo
```

## Новости

`emerge --news` получает официальную [RSS-ленту новостей Arch Linux](https://archlinux.org/feeds/news/). Состояние прочитанного хранится в `~/.cache/aura-emerge/news.state` (root не нужен).

```bash
emerge --news        # список, непрочитанные помечены [N]
emerge --news 3      # прочитать запись 3
emerge --news all    # пометить всё прочитанным
```

## Безопасность: сканирование PKGBUILD

Перед каждой установкой из AUR исходный `PKGBUILD` (и любой хук `.install`) загружается из cgit и сканируется в два слоя:

1. **AST** (`tree-sitter-bash`) - `curl|sh`, `base64`/`xxd | sh`, `source <(curl …)`, `eval "$(curl …)"`, `sudo`/`pkexec`/`doas`, inline-выполнение для **python / perl / ruby / node / lua**, `sh`/`bash`/`dash`/`ash` `-c` с подозрительной нагрузкой, подстановка команд на верхнем уровне, обходы через конкатенацию строк и т. д.
2. **Построчные эвристики** - `chmod 777`, «голые» IPv4, сторонние менеджеры пакетов (`npm install …`), paste-сайты, `.onion`, проверки на антипесочницу, известные IOC кампаний (Atomic Arch, openconnect-sso, `compromised.txt` 2018 года, …).

Находки бывают `Suspicious` или `ConfirmedIoc`. Срабатывание не блокирует жёстко: файл/строка, ссылка на cgit, `Continue anyway? [y/N]`.

**Также усилено:** текст RSS новостей Arch очищается (нельзя внедрить ANSI/bidi в терминал или `news.state`); файлы конфига/маски/env/world открываются с `O_NOFOLLOW`, поэтому символьную ссылку на конечный файл нельзя подсунуть через гонку; Rust-зависимости проходят через [`cargo deny`](https://github.com/EmbarkStudios/cargo-deny) (`make deny`).

Сборки AUR и ABS дополнительно ограничены [песочницей bwrap](#песочница) и аудитом архива пакета перед `pacman -U`.

<details>
<summary><h3>Тестирование (после установки)</h3></summary>

```bash
git clone https://github.com/Undercat037/aura-emerge.git
cd aura-emerge
emerge --install-pkgbuild ./scaner-test --ask
```
</details>


<details>
<summary><h2>Флаги</h2></summary>

| Флаг | Описание |
|------|----------|
| `-s` | Поиск пакетов |
| `-S`, `--searchdesc` | Поиск по описанию |
| `-sv` | Подробная информация о пакете |
| `-u` | Полное обновление системы (world не читает) |
| `--devel` | С `-u` - заодно пересобрать пакеты `-git`/`-hg`/`-svn`/`-bzr`, чей апстрим сдвинулся |
| `--check-devel` | Показать, у каких devel-пакетов изменился апстрим (без пересборки) |
| `-C` | Удалить пакеты (unmerge) |
| `-c` | Удалить сирот (depclean) |
| `-p` | Pretend (пробный прогон) |
| `-a` | Спросить перед действием |
| `--sudoloop` | Держать метку времени sudo «тёплой» весь запуск |
| `-1` | Установить как зависимость (oneshot) |
| `-v` | Подробный вывод |
| `-n` | Не переустанавливать, если уже есть (noreplace) |
| `--aur` | Принудительно AUR (зависимости только из AUR всегда разрешаются и собираются рекурсивно) |
| `--repos` | Только официальные репозитории |
| `--abs` | Сборка из ABS (`pkgctl`), в песочнице bwrap |
| `--no-sandbox` | Отключить bwrap для AUR/ABS |
| `--unshare-net-build` | Без сети во время build()/check()/package() |
| `--skippgp` | Пропустить проверки PGP (ABS) |
| `--autopgp` | Автоимпорт недостающих PGP-ключей (ABS) |
| `--edit` | Править PKGBUILD перед сборкой; после сохранения перегенерирует `.SRCINFO` |
| `--pkgbuild-view` | Показать/сравнить PKGBUILD, подтвердить перед сборкой (при отказе предлагает `$EDITOR`) |
| `--install-pkgbuild <PATH>` | Собрать+установить локальный каталог с PKGBUILD (сканер + bwrap) |
| `--scan` | Только аудит PKGBUILD/.install, без сборки/установки (AUR или `--install-pkgbuild`) |
| `--skip-srcinfo-regen` | С `--edit` - не перегенерировать `.SRCINFO` |
| `--sync` | Синхронизировать базы пакетов |
| `--refresh` | Принудительный `-Syy` (с `--sync` или `-u`) |
| `--info` | Информация о системе |
| `--news [N\|all]` | Новости Arch Linux |
| `--check-news [N\|all]` | Псевдоним `--news` |
| `--select` | Добавить в world без установки |
| `--deselect` | Убрать из world без удаления |
| `--prune` | Удалить пакеты, которых нет в world |
| `--regen` | Пересоздать кэш метаданных |
| `--regen-world` | Заново определить префиксы репозиториев в world |
| `--regen-sets <SET>` | Заново определить префиксы для одного пользовательского набора |
| `--regen-sort` | С `--regen-sets` - заодно отсортировать файл |
| `--regen-world-from-explicit` | Миграция: заполнить world из `pacman -Qeq` |
| `--err-install` | При `@world` заодно повторять записи `Err/` |
| `--resume` | Продолжить прерванную транзакцию |
| `--skipfirst` | Пропустить первый пакет при продолжении |
| `--undo` | Отменить последнюю установку/удаление |
| `--clean-source-cache` | Очистить постоянный кэш исходников VCS |
| `--batchinstall <FILE>` | Массовая установка из файла со списком пакетов |
| `--list-sets` | Список доступных `@наборов` |
| `--exclude <ATOM>` | Исключить пакет из этого запуска (повторяемый/через запятую) |
| `--keep-going` | Не останавливать пакетную операцию на первой ошибке; сводка в конце |
| `--revdep-rebuild` | Найти и починить бинарники, линкующиеся с отсутствующей библиотекой |
| `-t`, `--tree` | Показать план как дерево зависимостей |
| `-D`, `--deep[=N]` | С `-t`: вложенность на N уровней, а без значения - на все |
| `--ignore-default-opts` | Игнорировать `EMERGE_DEFAULT_OPTS` из make.conf на этот запуск |
| `--debug` | Показывать вывод makepkg/компилятора вживую (также `AE_DEBUG=1`) |
| `--log <PATH>` | Записать лог сессии (команда, строки emerge.log, полный вывод сборки) для баг-репортов |
| `-h` / `--help` | Справка |
| `-V` / `--version` | Версия |

Страница руководства `emerge(1)` генерируется из CLI через `clap_mangen` (`emerge --gen-manpage`), поэтому не может разойтись с `--help`.
</details>

## Автор/Лицензия
Copyright (C) 2026 **Undercat037**
Лицензия GPL-3.0
