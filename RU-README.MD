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
sudo cp target/release/aura-emerge /usr/bin/emerge
sudo ln -sf /usr/bin/emerge /usr/bin/portageq
emerge --gen-completions bash | sudo tee /usr/share/bash-completion/completions/emerge
emerge --gen-completions zsh  | sudo tee /usr/share/zsh/site-functions/_emerge
emerge --gen-completions fish | sudo tee /usr/share/fish/vendor_completions.d/emerge.fish
emerge --gen-manpage | sudo tee /usr/share/man/man1/emerge.1 >/dev/null
```
</details>

**Зависимости:** `git`, `bubblewrap` (песочница сборки для AUR/ABS)
**Опционально:** `devtools` (поддержка `--abs`), `gnupg` (`--autopgp`, автоимпорт PGP-ключей)

## Использование

```bash
# Установка
emerge neovim                       # официальные репозитории, при неудаче - AUR
emerge neovim-git --aur             # только AUR (зависимости только из AUR собираются рекурсивно)
emerge neovim --repos               # только официальные репозитории, AUR не трогать
emerge neovim --abs                 # сборка из ABS (pkgctl) в песочнице
emerge -p neovim                    # пробный прогон
emerge -a neovim                    # спросить перед действием
emerge -1 htop                      # поставить как зависимость (в world не попадает)

# Удаление
emerge -C neovim                    # удалить пакет (unmerge)
emerge -c                           # удалить сирот (depclean)
emerge --prune                      # удалить пакеты, которых нет в world

# Обновление
emerge --sync                       # обновить базы пакетов
emerge -u                           # полное обновление (репозитории + AUR)
emerge -u --devel                   # ...и пересобрать -git/-hg/-svn/-bzr, чей апстрим ушёл вперёд
emerge -u --exclude linux --keep-going

# World и наборы
emerge @world                       # поставить всё из world, чего ещё нет
emerge @game-kit neovim             # пользовательский набор + обычный пакет
emerge --select neovim              # добавить в world без установки
emerge --deselect neovim            # убрать из world без удаления

# Поиск и информация
emerge -s neovim                    # поиск: репозитории, затем ABS, затем AUR
emerge -S neovim                    # поиск по описанию
emerge -sv neovim                   # подробная информация о пакете
emerge --info                       # информация о системе
emerge --news                       # новости Arch Linux

# Безопасность
emerge neovim-git --aur --pkgbuild-view      # показать/сравнить PKGBUILD перед сборкой
emerge neovim-git --aur --edit               # сначала отредактировать PKGBUILD
emerge --scan neovim-git --aur               # только аудит, без сборки
emerge --install-pkgbuild ~/src/my-pkg       # собрать локальный PKGBUILD через сканер и песочницу

# Восстановление
emerge --resume                     # продолжить прерванную транзакцию
emerge --undo                       # отменить последнюю установку/удаление
emerge --revdep-rebuild             # починить бинарники с битыми ссылками на библиотеки
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
EMERGE_DEFAULT_OPTS="--pkgbuild-view --unshare-net-build"

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

**Ключи:** `EMERGE_DEFAULT_OPTS`, `FEATURES`, `CFLAGS`, `CXXFLAGS`, `CPPFLAGS`, `LDFLAGS`, `RUSTFLAGS`, `MAKEFLAGS`, `NINJAFLAGS`, `BUILDENV`, `OPTIONS`.
**Синтаксис:** как в bash - `KEY="..."` или `KEY=(a b c)`. Строки и списки взаимозаменяемы, значения раскрываются оболочкой (`$(nproc)` и `$CFLAGS` работают; одинарные кавычки оставят `$` как есть).

Токены `FEATURES` (`-name` выключает токен) переводятся в аналоги makepkg. То, что вы сами указали в `BUILDENV`/`OPTIONS`, главнее.

| `FEATURES`   | makepkg             |
|--------------|---------------------|
| `ccache`     | `BUILDENV` `ccache` |
| `distcc`     | `BUILDENV` `distcc` |
| `test`       | `BUILDENV` `check`  |
| `nostrip`    | `OPTIONS` `!strip`  |
| `splitdebug` | `OPTIONS` `debug`   |

`EMERGE_DEFAULT_OPTS` принимает только флаги-модификаторы (`--ask`, `--verbose`, `--devel`, `--tree`, `--searchdesc`, `--exclude=<atom>`, ...), так что превратить каждый запуск в `--unmerge` нельзя. Конфликтующие пары отклоняются, если заданы обе: `--aur`/`--abs`, `--aur`/`--repos`, `--no-sandbox`/`--unshare-net-build`, `--skippgp`/`--autopgp`, `--select`/`--deselect`. Если значение из конфига конфликтует с командной строкой, оно отбрасывается. `--abs` и `--repos` **не** конфликтуют.

`emerge --info` показывает, что было прочитано, и итоговое значение каждого переопределения. [**→ Пример make.conf**](misc/make.conf)

### `package.mask`

Пакеты, которые эта машина не должна ставить никогда: проверка срабатывает даже для транзитивных зависимостей из AUR и даже при запросе по имени. Замаскированный пакет жёстко останавливает запрос - без вопросов и без флага для обхода.

`/etc/portage/package.mask` - файл или каталог (читается каждый обычный файл по порядку имён; файлы с точки пропускаются; символьные ссылки отклоняются). Одна запись на строку, допускаются `*` и префикс репозитория, в конце можно добавить `# комментарий`.

```
ayugram-desktop-bin            # конкретный пакет, из любого источника
aur/*-bin                      # никогда не ставить -bin пакеты из AUR
*-git                          # на этой машине без devel-пакетов
extra/nano                     # конкретный пакет из конкретного репозитория
```

Это не то же самое, что `--exclude`: маска действует постоянно, `--exclude` - на один запуск.

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

Для пакетов `-git`/`-hg`/`-svn` `SRCDEST` по умолчанию - `~/.cache/aura-emerge/sources` (или `$XDG_CACHE_HOME/aura-emerge/sources`), поэтому makepkg делает инкрементальный fetch вместо полного клонирования. Если `SRCDEST` задан в `makepkg.conf`, используется он. Очистить кэш: `emerge --clean-source-cache`.

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

`/etc/portage/sets/<name>` (или `<name>.set`) вызывается как `@<name>`. Один пакет на строку, комментарии `#` допускаются.

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

Откатывает последнюю успешную установку или удаление (один шаг истории):

- **после установки** - удаляет только пакеты, которые в этом запуске были новыми;
- **после удаления** - переустанавливает из записанного источника (`Err/`/`abs/` только перечисляются);
- **после `-u`** - не поддерживается (нет снимка версий).

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
2. **Построчные эвристики:** `chmod 777`, «голые» IPv4, сторонние менеджеры пакетов (`npm install …`), paste-сайты, `.onion`, проверки на антипесочницу, известные IOC кампаний (Atomic Arch, openconnect-sso, `compromised.txt` 2018 года, …).

Находки бывают `Suspicious` или `ConfirmedIoc`. Срабатывание не блокирует жёстко: вы получаете файл/строку, ссылку на cgit и `Continue anyway? [y/N]`.

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
| `--no-sandbox` | Отключить bwrap; обычный `makepkg -si` |
| `--unshare-net-build` | Сеть только для загрузки/`prepare()`; `build()`/`check()`/`package()` - без сети |

Если `bubblewrap` не установлен, сборка отклоняется.

**Третий барьер перед установкой:** готовый `*.pkg.tar.*` проверяется через `bsdtar` на setuid/setgid, скрипты `.INSTALL` и чувствительные пути (хуки alpm, юниты systemd, sudoers, udev, …). `.INSTALL` повторно сканируется сканером PKGBUILD. С `-a` нужно подтвердить; с `--noconfirm` - предупреждение и продолжение.

Установка зависимостей (`pacman -S`) и финальный `pacman -U` выполняются **вне** песочницы - они не запускают код PKGBUILD.

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
|------|----------|
| `-s` | Поиск пакетов |
| `-S`, `--searchdesc` | Поиск по описанию |
| `-sv` | Подробная информация о пакете |
| `-u` | Полное обновление системы (world не читает) |
| `--devel` | С `-u`: заодно пересобрать `-git`/`-hg`/`-svn`/`-bzr`, чей апстрим ушёл вперёд |
| `--check-devel` | Показать devel-пакеты с изменениями в апстриме (без пересборки) |
| `-C` | Удалить пакеты (unmerge) |
| `-c` | Удалить сирот (depclean) |
| `-p` | Pretend (пробный прогон) |
| `-a` | Спросить перед действием |
| `--sudoloop` | Держать метку времени sudo «тёплой» весь запуск |
| `-1` | Установить как зависимость (oneshot) |
| `-v` | Подробный вывод |
| `-n` | Не переустанавливать, если уже есть (noreplace) |
| `--aur` | Принудительно AUR (зависимости только из AUR всегда собираются рекурсивно) |
| `--repos` | Только официальные репозитории |
| `--abs` | Сборка из ABS (`pkgctl`) в песочнице bwrap |
| `--no-sandbox` | Отключить bwrap для AUR/ABS |
| `--unshare-net-build` | Без сети во время build()/check()/package() |
| `--skippgp` | Пропустить проверки PGP (ABS) |
| `--autopgp` | Автоимпорт недостающих PGP-ключей (ABS) |
| `--edit` | Править PKGBUILD перед сборкой; после сохранения перегенерирует `.SRCINFO` |
| `--pkgbuild-view` | Показать/сравнить PKGBUILD и подтвердить перед сборкой (при отказе предлагает `$EDITOR`) |
| `--install-pkgbuild <PATH>` | Собрать и установить локальный каталог с PKGBUILD (сканер + bwrap) |
| `--scan` | Только аудит PKGBUILD/.install, без сборки/установки (AUR или `--install-pkgbuild`) |
| `--skip-srcinfo-regen` | С `--edit`: не перегенерировать `.SRCINFO` |
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
| `--regen-sort` | С `--regen-sets`: заодно отсортировать файл |
| `--regen-world-from-explicit` | Миграция: заполнить world из `pacman -Qeq` |
| `--err-install` | С `@world`: заодно повторять записи `Err/` |
| `--resume` | Продолжить прерванную транзакцию |
| `--skipfirst` | С `--resume`: пропустить первый пакет |
| `--undo` | Отменить последнюю установку/удаление |
| `--clean-source-cache` | Очистить постоянный кэш исходников VCS |
| `--batchinstall <FILE>` | Массовая установка из файла со списком пакетов |
| `--list-sets` | Список доступных `@наборов` |
| `--exclude <ATOM>` | Исключить пакет из этого запуска (повторяемый/через запятую) |
| `--keep-going` | Не останавливать пакетную операцию на первой ошибке; сводка в конце |
| `--revdep-rebuild` | Найти и починить бинарники, линкующиеся с отсутствующей библиотекой |
| `-t`, `--tree` | Показать план как дерево зависимостей |
| `-D`, `--deep[=N]` | С `-t`: вложенность на N уровней, без значения - на все |
| `--ignore-default-opts` | Игнорировать `EMERGE_DEFAULT_OPTS` из make.conf на этот запуск |
| `--debug` | Показывать вывод makepkg/компилятора вживую (также `AE_DEBUG=1`) |
| `--log <PATH>` | Записать лог сессии (команда, строки emerge.log, полный вывод сборки) для баг-репортов |
| `-h`, `--help` | Справка |
| `-V`, `--version` | Версия |

Страница руководства `emerge(1)` генерируется из CLI (`emerge --gen-manpage`), поэтому не может разойтись с `--help`.
</details>

## Автор / Лицензия
Copyright (C) 2026 **Undercat037**
Лицензия GPL-3.0
