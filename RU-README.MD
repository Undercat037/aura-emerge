<div align="center">
  <img src="misc/logo/slash-logo-128x128.svg" width="128" alt="aura-emerge logo">

# Aura-Emerge Secure Package Manager

[![AE_Site](https://img.shields.io/badge/AE_Site-333333?style=flat-square&logo=bookstack&logoColor=FFFFFF)](https://undercat037.github.io/aura-emerge/)
[![AUR](https://img.shields.io/badge/AUR-333333?style=flat-square&logo=arch-linux&logoColor=1793D1)](https://aur.archlinux.org/packages/aura-emerge)
[![GitHub](https://img.shields.io/badge/GitHub-333333?style=flat-square&logo=github&logoColor=FFFFFF)](https://github.com/Undercat037/aura-emerge)
[![GitLab](https://img.shields.io/badge/GitLab-333333?style=flat-square&logo=gitlab&logoColor=FC6D26)](https://gitlab.com/Undercat037/aura-emerge)

[en](README.MD) | [ua](UA-README.MD) | ru
</div>

> Автономный менеджер пакетов `emerge` в стиле Gentoo для Arch Linux. Ставит пакеты из официальных репозиториев, AUR и ABS; перед сборкой сканирует PKGBUILD на признаки атак на цепочку поставок; недоверенные шаги сборки выполняет в песочнице `bwrap`.

Пакеты учитываются в файле `world` - установил один раз, отслеживается всегда.

**Разделы:** [Установка](#установка) · [Примеры использования](#примеры-использования) · [Конфигурация](#конфигурация) · [Работа с пакетами](#работа-с-пакетами) · [Репозитории](#репозитории) · [Безопасность](#безопасность) · Флаги (в конце)

## Установка

```bash
paru -S aura-emerge          # через AUR-помощник
```

или обычным makepkg:

```bash
git clone https://aur.archlinux.org/aura-emerge.git
cd aura-emerge
makepkg -si
```

<details>
<summary><h3>Сборка из исходного кода (нестабильно, beta-версии)</h3></summary>

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
emerge --gen-completions bash | sudo tee /usr/share/bash-completion/completions/emerge >/dev/null
emerge --gen-completions zsh  | sudo tee /usr/share/zsh/site-functions/_emerge >/dev/null
emerge --gen-completions fish | sudo tee /usr/share/fish/vendor_completions.d/emerge.fish >/dev/null
emerge --gen-manpage | sudo tee /usr/share/man/man1/emerge.1 >/dev/null
sudo install -Dm644 misc/eselect.bash-completion /usr/share/bash-completion/completions/eselect
sudo install -Dm644 misc/_eselect /usr/share/zsh/site-functions/_eselect
sudo install -Dm644 misc/eselect.fish /usr/share/fish/vendor_completions.d/eselect.fish
```
</details>

> **Зависимости:** `git`, `sudo`, `bubblewrap` (песочница сборки AUR/ABS), `pacman` (libalpm, makepkg), `gnupg` (импорт ключей для внутренней реализации `pacman-key` и для `--autopgp` с ABS).

> **Опционально:** `devtools` (поддержка `--abs`).

## Примеры использования

```bash
# Установка
emerge neovim                       # официальные репозитории, при неудаче — AUR
emerge neovim --nodeps              # установить пакет, не устанавливая зависимости
emerge neovim-git --aur             # только AUR (зависимости только из AUR собираются рекурсивно)
emerge neovim --repos               # только официальные репозитории
emerge neovim --abs                 # сборка из ABS (pkgctl) в песочнице
emerge aur/neovim-git abs/nano cachyos-extra-v3/neovim   # источник каждого пакета — префиксом
emerge -pt neovim                   # пробный прогон, план деревом
emerge -a neovim                    # спросить перед действием
emerge -1 htop                      # как зависимость (в world не попадает)
emerge -at aur/setrixtui abs/nano @fonts --jobs=7 --jobsa=2 # несколько пакетов сразу: по 7 из репозиториев, по 2 сборки из AUR/ABS

# Удаление
emerge -C neovim                    # удалить пакет (--unmerge)
emerge -c                           # удалить сирот (--depclean)
emerge -cp --show-protected         # сироты, а также те, что удерживаются через world, package.protect или выбранный профиль
emerge --prune                      # удалить всё, чего нет в world

# Обновление
emerge --sync                       # обновить базы пакетов
emerge -u                           # полное обновление (репозитории + AUR)
emerge -u --devel                   # ...и пересобрать -git/-hg/-svn/-bzr, чей upstream ушёл вперёд
emerge -u --exclude linux --keep-going # исключить пакет `linux` из обновления

# World и наборы
emerge @world                       # установить всё из world, чего ещё нет
emerge @desktop neovim              # свой набор + обычный пакет
emerge --select neovim              # добавить в world без установки
emerge --deselect neovim            # убрать из world без удаления

# Поиск и информация
emerge -s neovim                    # поиск: репозитории => ABS => AUR
emerge -S neovim                    # поиск по описанию
emerge -sv neovim                   # подробно
emerge -s abs/nano                  # только ABS (префиксы aur/ и <repo>/ тоже работают)
emerge --info                       # информация о системе
emerge --news                       # новости Arch Linux

# Безопасность
emerge neovim-git --aur --pkgbuild-view   # показать/сравнить PKGBUILD перед сборкой
emerge neovim-git --aur --edit            # сначала отредактировать PKGBUILD
emerge --scan neovim-git --aur            # только аудит, без сборки
emerge --usepkgbuild ~/src/my-pkg         # локальный PKGBUILD через сканер и песочницу
emerge --usepkg ./foo-1.0-1-x86_64.pkg.tar.zst  # готовый архив (pin + аудит)

# Без установки
emerge -B neovim --aur              # только собрать архивы
emerge -f neovim --aur --outdistfiles ~/ae-out   # только скачать всё дерево в выбранную директорию
emerge --usepkg ~/ae-out            # установить все архивы из каталога одной транзакцией

# Восстановление
emerge --resume                     # продолжить прерванную транзакцию
emerge --undo                       # отменить последнюю установку/удаление/depclean
emerge --revdep-rebuild             # починить бинарники с битыми ссылками на .so
emerge --aelist                     # файлы, оставленные AE Force
```

Полная таблица флагов — в конце страницы.

## Конфигурация

Всё настраивается в `/etc/portage/`. Конфиг только системный и принадлежит root.

| Путь | Назначение |
|------|------------|
| `make.conf` | Главный конфиг: флаги по умолчанию, цвета, флаги сборки, `FEATURES` |
| `package.mask`, `package.unmask` | Что никогда не устанавливать / исключения |
| `package.protect`, `package.freeze` | Что нельзя удалять / что не обновлять |
| `package.env`, `env/` | Флаги сборки для отдельных пакетов |
| `make.profile` | Профиль (символическая ссылка) |
| `world`, `sets/` | Явно установленные пакеты и собственные наборы |
| `repos.conf`, `ae-repos.conf` | Список репозиториев (`FEATURES="repos"`) и секции `ae-*` для `pacman.conf` |
| `hooks/` | Собственные alpm-хуки |
| `resume.state`, `lastaction.state` | Состояние для `--resume` и `--undo` |

Прочее: `/var/lib/aura-emerge/repos/` — локальные репозитории, `/var/log/emerge.log` — журнал операций, `~/.cache/aura-emerge/` — кеши (PKGBUILD-view, devel, новости, сборки), `/var/db/repos/portage/profiles/` — база профилей.

### make.conf

```bash
# Флаги по умолчанию. Подставляются перед набранными (командная строка главнее).
# Отключить на один запуск: --ignore-default-opts.
EMERGE_DEFAULT_OPTS="--jobs=8 --tree --deep"

# Цвета строк статуса (роли: ok, warn, error, info, special).
COLORS="ok=#FFB3AE warn=#E7BDB9 error=#FDB1AC info=#E7BDB9 special=#cba6f7"

# Флаги сборки (см. ниже)
CFLAGS="-march=native -O2 -pipe"
CXXFLAGS="$CFLAGS"
RUSTFLAGS="-C target-cpu=native"
MAKEFLAGS="-j$(nproc)"
NINJAFLAGS="-j$(nproc)"
OPTIONS=(strip docs !debug lto)
BUILDENV=(!distcc color check !sign)

# Переключатели только для aura-emerge
FEATURES="localrepo"
```

Синтаксис как в bash: `KEY="..."`, `KEY='...'` (буквально, без раскрытия) или `KEY=(a b c)`. В двойных кавычках и без кавычек работают `$(nproc)` и `$CFLAGS`. Файл читается с `O_NOFOLLOW`: символическая ссылка отклоняется.

| Ключ | Назначение |
|------|------------|
| `EMERGE_DEFAULT_OPTS` | Флаги, подставляемые в каждый запуск |
| `COLORS` | Палитра статусного вывода |
| `FEATURES` | Переключатели aura-emerge |
| `EMERGE_FREEZE` | То же, что `package.freeze`, одной строкой |
| `CFLAGS` `CXXFLAGS` `CPPFLAGS` `LDFLAGS` `RUSTFLAGS` `MAKEFLAGS` `NINJAFLAGS` `BUILDENV` `OPTIONS` | Флаги сборки AUR/ABS |

**`EMERGE_DEFAULT_OPTS`** принимает только флаги-модификаторы (`--ask`, `--verbose`, `--devel`, `--tree`, `--deep`, `--jobs=N`, `--jobsa=N`, `--searchdesc`, `--exclude=<atom>` и т. д.), так что сделать из каждого запуска `--unmerge` нельзя. Противоречивые пары отклоняются, если заданы обе: `--aur`/`--abs`, `--aur`/`--repos`, `--no-sandbox`/`--unshare-net-build`, `--skippgp`/`--autopgp`, `--select`/`--deselect`. Значение из конфига, противоречащее командной строке, отбрасывается. `--abs` и `--repos` друг другу **не** противоречат.

**`COLORS`** — токены `роль=цвет`; цвет — `#RRGGBB` или имя (`red`, `bright-cyan`, …). Последнее упоминание роли побеждает, неизвестные роли и цвета пропускаются с предупреждением.

**`FEATURES`** — только для aura-emerge, в makepkg никогда не передаётся (ccache, check, strip задавайте в `BUILDENV` / `OPTIONS`). `-name` выключает токен, побеждает последнее упоминание. Токены:

| Токен | Что включает |
|-------|--------------|
| `localrepo` | локальные репозитории `ae-aur` / `ae-abs` / `ae-local` ([подробнее](#локальные-репозитории)) |
| `repos` | Portage-style `repos.conf` ([подробнее](#reposconf)) |

Неизвестные токены читаются и показываются в `--info`, но ничего не делают. `emerge --info` печатает действующие `FEATURES` и значения, перекрывающие makepkg. [**=> Пример make.conf**](misc/make.conf)

#### Gentoo => aura-emerge

| Gentoo (`make.conf`) | aura-emerge (`make.conf`) |
|----------------------|---------------------------|
| `CFLAGS`, `CXXFLAGS`, `RUSTFLAGS` | так же |
| `MAKEOPTS="-j$(nproc)"` | `MAKEFLAGS="-j$(nproc)"` |
| `USE="lto"` | `OPTIONS=(... lto)` |
| `FEATURES="ccache"` | `BUILDENV=(... ccache)` |
| `FEATURES` (функции Portage) | `FEATURES` только для AE: `localrepo`, `repos` |

USE-флагов в Arch нет, поэтому их роль частично выполняет `OPTIONS`. Справка: [флаги оптимизации GCC](https://gcc.gnu.org/onlinedocs/gcc/Optimize-Options.html), [параметры кодогенерации rustc](https://doc.rust-lang.org/rustc/codegen-options/index.html).

### Флаги сборки и makepkg.conf

Флаги сборки задаются только в `make.conf` (и точечно в [`package.env`](#packageenv)). Править `/etc/makepkg.conf` ради aura-emerge не нужно.

Перед каждой сборкой AUR/ABS emerge сам генерирует временный `makepkg.conf`: сначала он подключает системный, затем перекрывает его ключами из `make.conf`, и этот файл передаётся в makepkg через `--config`. Поэтому:

- системный `/etc/makepkg.conf` — лишь база для ключей, которых в `make.conf` нет (`PKGEXT`, сжатие, `DLAGENTS` и др.); при совпадении ключей всегда побеждает `make.conf`;
- если в `make.conf` нет ни одного ключа сборки, makepkg остаётся без изменений;
- в песочнице пользовательский `~/.config/pacman/makepkg.conf` не действует (`$HOME` пуст); без песочницы (`--no-sandbox`) makepkg подключает его *после* сгенерированного, и совпавший ключ побеждает make.conf — emerge предупреждает об этом.

По безопасности: значения экранируются при генерации (`"`, `` ` ``, `\`; значение в одинарных кавычках не раскрывается вообще), в `BUILDENV` / `OPTIONS` допустимы только токены из букв, цифр и `!_-+.` (иначе предупреждение, и ключ остаётся как в системном файле). Сгенерированный файл лежит в приватном (`0700`) служебном каталоге вне дерева сборки, чтобы `prepare()` / `build()` не могли его переписать, и удаляется после сборки.

### package.env

Флаги сборки для отдельных пакетов. Запись — атом и один или несколько env-файлов из `/etc/portage/env/`. `/etc/portage/package.env` — файл или каталог.

```
# /etc/portage/package.env/neovim
abs/neovim  lto.conf            # префикс репозитория и '*' работают как в package.mask
aur/*-git   no-lto  fast        # несколько env применяются в указанном порядке
```

```bash
# /etc/portage/env/lto.conf  (синтаксис make.conf; учитываются только ключи сборки)
CFLAGS="-march=znver3 -O2 -pipe"
MAKEFLAGS="-j16"
OPTIONS="strip docs !debug lto"
```

- **Наложение:** `make.conf` => подходящие записи `package.env` => каждый env-файл по порядку. Побеждает слой, задавший ключ последним; ключ заменяется целиком, а не сливается. Сборка сообщает, какой слой победил.
- **Совпадение:** по pkgbase или любому pkgname.
- **Область:** только сборки AUR/ABS. `EMERGE_DEFAULT_OPTS` в env-файле игнорируется с предупреждением.
- **Ошибки:** символическая ссылка, некорректная запись или отсутствующий env-файл пропускаются с предупреждением (файл и строка), остальное применяется.

### package.mask и package.unmask

`package.mask` — пакеты, которые эта машина не должна устанавливать никогда. Проверка срабатывает на любом пути: запрос по имени, транзитивная зависимость из AUR, провизия world, `--select`/`-R`/`-u`. Замаскированный пакет — жёсткая остановка, без вопроса и без флага для обхода; обойти можно только записью в `package.unmask`.

`/etc/portage/package.mask` — файл или каталог (читается каждый обычный файл по порядку имён; файлы с точкой в начале пропускаются; символические ссылки отклоняются). Одна запись на строку, допустимы `*`, префикс репозитория и комментарий `# …` в конце.

```
ayugram-desktop-bin            # конкретный пакет, из любого источника
aur/*-bin                      # никогда не ставить -bin из AUR
*-git                          # на этой машине без devel-пакетов
extra/nano                     # конкретный пакет из конкретного репозитория
```

Префикс репозитория учитывается везде, где источник известен (`extra/nano` срабатывает только для `extra`, `aur/foo-git` — только для AUR). Версионные атомы Portage (`=cat/foo-1.2`, `>=cat/foo`) не поддерживаются: сопоставление идёт по имени, такие записи выводятся с предупреждением и пропускаются. Маска действует постоянно, в отличие от `--exclude`, который действует на один запуск.

`package.unmask` (тот же синтаксис) перечисляет исключения: пакет, подпадающий под запись, проходит, даже если его блокирует `package.mask` или маска активного профиля. Root-helper это тоже учитывает.

```
aur/some-git                   # разрешён, хотя *-git замаскированы выше
```

### package.protect и package.freeze

**`package.protect`** — пакеты, которые нельзя удалять (синтаксис как в `package.mask`). Действует на `-C`, `-c`, `--prune` и удаления из-за конфликтов. `--deselect` всё ещё может убрать имя из world, но пакет останется установленным, и depclean его не тронет. Пакеты `@system` из профиля защищены так же.

**`package.freeze`** (и необязательный `EMERGE_FREEZE` в `make.conf`) держит пакеты на установленной версии во время `-u`, включая пересборки AUR/ABS. Строже, чем `IgnorePkg` в pacman: действует и на выборочный `emerge -u pkg`.

### Профили

`/etc/portage/make.profile` — символическая ссылка на каталог профиля (раскладка Portage). `--list-profiles` показывает дерево из `/var/db/repos/portage/profiles`; `parent` вида `repo:path` ищется в `/var/db/repos/<repo>/profiles/`. Профиль может содержать:

- `parent` — цепочки раскрываются вглубь;
- `package.profile` — `*` означает стартовый набор `@system` (старое имя `packages` тоже читается);
- `package.mask` — `-atom` или `!atom` снимает маску, унаследованную от родителя;
- `package.protect`, `make.defaults`.

`emerge @system` устанавливает системный набор профиля; все записи `package.profile` защищены от depclean так же, как world.

```bash
emerge --list-profiles
sudo emerge --set-profile 12
# или через eselect-ссылку: eselect profile set 12
```

`/usr/bin/eselect` => `emerge` даёт минимальный `eselect` (только `eselect profile list|set|show`). Для `--list-profiles` и `--set-profile` он не нужен.

### world и наборы

**`/etc/portage/world`** — список всех явно установленных пакетов. Записи добавляются при установке, убираются при удалении и редактируются вручную (по одной на строку).

```
core/nano
extra/firefox
aur/ayugram-desktop-bin
abs/nano
local/my-pkg
chaotic-aur/visual-studio-code-bin
```

**Префиксы:** название официального репозитория, `aur/`, `abs/`, `local/` (`--usepkgbuild` и `--usepkg`), `Err/` (источник неизвестен).

> **Про `Err/`:** установленный пакет, происхождение которого определить не удалось, либо запись `aur/` / `abs/`, которая перестала разрешаться. `--regen-world` и `--regen-sets` сохраняют известный префикс, а не откатываются к `Err/`. Во время `emerge @world` такие записи только перечисляются; установить их можно с `--err-install`.

Если в пакетной установке одно имя не найдено, остальные всё равно устанавливаются; отсутствующие выводятся в конце, код выхода ненулевой.

**У `@world` два значения:**

| Команда | Действие |
|---------|----------|
| `emerge @world` | *Провизия:* ставит то, что есть в world, но ещё не установлено. Держите world в dotfiles, клонируйте на новую машину, запускайте |
| `emerge -u` / `emerge -u @world` | *Обновление:* обновляет всё установленное (репозитории + AUR); world не читает |
| `emerge --sync` | Только обновление баз (`-Sy` / `-Syy`) |

**Собственные наборы.** `/etc/portage/sets/<name>` (или `<name>.set`) вызывается как `@<name>`. Один пакет на строку, комментарии `#` допустимы; строка `@other` вкладывает другой набор (рекурсивно, циклы безопасны). Установка `@name` записывает `@name` в world, поэтому члены набора защищены от depclean, пока их не уберут из файла набора. Если существуют и `game-kit`, и `game-kit.set`, `@game-kit` отклоняется как неоднозначный.

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
emerge -s @game-kit                   # откуда берётся каждый пакет: repo / aur / not found
emerge --batchinstall pkgs.txt        # тот же формат, но файл где угодно и без регистрации
```

`@world` и `@preserved-rebuild` — действия, поэтому `-s @world` и `-s @preserved-rebuild` дают ошибку.

## Работа с пакетами

### Источники и префиксы

Без префикса emerge ищет в официальных репозиториях, а при неудаче — в AUR. Префикс выбирает источник: `aur/<name>`, `abs/<name>`, `<repo>/<name>`. Префиксы работают в командах, в `world`, в наборах и в `-s`. `--aur`, `--repos`, `--abs` задают источник для всего запуска; зависимости, которые есть только в AUR, собираются рекурсивно.

### Параллельные jobs и прогресс

`--jobs=N` ставит до N пакетов из официальных репозиториев одновременно, `--jobsa=N` — до N сборок AUR/ABS (по умолчанию 1 и 1). `--load-average=X` не запускает новые сборки, пока нагрузка за минуту выше X (но никогда, если больше ничего не работает). `--nospinner` убирает живую строку Jobs. Значения по умолчанию удобно держать в `EMERGE_DEFAULT_OPTS`.

```
>>> Emerging (3 of 12, 16%) noctalia-5.1.0-1.1
>>> Jobs: 26 of 26 complete  2m53s                         Load avg: 2.49, 3.11, 2.17
```

Во время тихой сборки каждые 30 с печатается heartbeat (pid, RSS: `still building … — process alive`); если процесс убит сигналом (OOM/SIGKILL), об этом сообщается явно. Если helper отклонил один пакет из пачки (например, он замаскирован), пачка повторяется по одному, чтобы остальные всё равно установились. `--debug` (или `AE_DEBUG=1`) показывает вывод компилятора вживую.

### План деревом: `-t` и `--deep`

Показывает план (с `-p` или перед настоящей установкой) деревом зависимостей. По умолчанию — один уровень; `--deep` идёт до дна, `--deep=N` — на N уровней.

```
[ebuild  N ] noctalia-5.1.0-1.1
[ebuild  N ] `-- libqalculate-5.12.0-1.1
[ebuild  N ]   `-- sdbus-cpp-2.3.1-1.1
```

Зависимость, не названная ни в чьём «Depends On» (разрешена через `provides` или глубже `--deep`), всё равно показывается, но без родителя. Для чистой установки через `--aur` транзитивные зависимости только из AUR на шаге плана пока не разрешаются, поэтому дерево там плоское.

### `--exclude` и `--keep-going`

`--exclude <atom>` (повторяемый или через запятую) исключает пакет из запуска: установки, `-u`, `@world`, `--depclean`/`--prune` (защищает от удаления) и `--revdep-rebuild`.

`--keep-going` не останавливает пакетную операцию (`@world`, `-u`, `--batchinstall`, несколько пакетов) на первой ошибке: сбои запоминаются, в конце печатается итог и возвращается ненулевой код. Следующий `--resume` повторит только то, что упало.

```bash
emerge -u --exclude linux --exclude aur/ayugram-desktop-bin
emerge -u --keep-going
emerge --resume
```

### Конфликты (блокеры)

Установленные пакеты, блокирующие входящий, показываются до начала установки, как блокеры Portage:

```
[ebuild  N    ] pipewire-jack-1:1.4.2-1
[conflict     ] jack2-1.9.22-1 ("jack2" is blocking pipewire-jack-1:1.4.2-1)
```

Проверка охватывает все источники запуска: план из репозиториев (libalpm), пакеты AUR и их AUR-зависимости, пакеты ABS (по `.SRCINFO`: `provides`/`conflicts`) и голый `@world`. Источники проверяются и **друг против друга**: если два пакета одного запуска блокируют друг друга (скажем, из репозитория и из AUR), это показывается как `[blocks]`, и ничего не устанавливается. Для установленных блокеров нужен один `Yes` (спрашивается всегда); сначала перечисляются пакеты, которым они нужны, а записи `package.protect` не удаляются никогда. То, что предварительная проверка не увидела (например, нет сети для `.SRCINFO`), всё равно ловится во время установки пакета.

### Файлы без владельца: AE Force

Если установка перезаписала бы файл, которым не владеет ни один пакет, emerge спрашивает `AE Force: Create .aenew for conflict files?`. «Да» оставляет ваш файл на месте, а версию из пакета сохраняет как `<file>.aenew`.

| Флаг | Что происходит |
|------|----------------|
| `--aeforce` | без вопроса; ваш файл переносится в `<file>.aeforce`, новый занимает его место |
| `--noaefile` | без вопроса; файлов `.ae*` не остаётся: старый сохранён, новый отброшен (с `--aeforce`: старый отброшен, новый сохранён) |

`<file>.aepark` остаётся после прерванной установки; если имя занято, добавляется суффикс `.N`. `emerge --aelist` перечисляет все такие файлы, `emerge --aeclean [new|old|park|all]` удаляет их после вопроса (без аргумента = `all`).

### Хуки

libalpm выполняет каждый `*.hook` при установке, удалении, `-U` и `-Su`, как и pacman (depmod, DKMS, mkinitcpio/dracut, кеши fontconfig и иконок, `grub-mkconfig`, …). При совпадении имён побеждает более ранний каталог:

```
/usr/share/libalpm/hooks/    хуки из пакетов
/etc/pacman.d/hooks/         хуки администратора
/etc/portage/hooks/          собственные хуки aura-emerge (формат alpm-hooks(5))
```

Все три регистрируются всегда, даже без `HookDir` в `pacman.conf`. Ход выполнения: `>>> Running pre-transaction hooks...` / `>>> (2 of 4) Install DKMS modules`.

### Обновление

`emerge -u` обновляет репозитории и AUR (world не читает). Две вещи, которых не хватает обычному обновлению:

**Devel-пакеты (`-git`, `-hg`, `-svn`, `-bzr`).** Версия в AUR меняется, только когда мейнтейнер перезаливает пакет, поэтому `-u` часто не видит изменений в upstream. `-u --devel` (обновление) и `--check-devel` (только отчёт) закрывают этот пробел: для каждого установленного devel-пакета берут URL `git+` из PKGBUILD, делают `git ls-remote` (клон не нужен) и сравнивают с хешем прошлого раза (`~/.cache/aura-emerge/devel.state`). Пока только git; `-hg`/`-svn`/`-bzr` выводятся как «couldn't determine», а не пропускаются молча.

**Сломанные ссылки на библиотеки.** В Arch нет учёта preserved-libs, поэтому есть два инструмента:

- `@preserved-rebuild` собирает *объявленные* зависимости всех установленных пакетов (`pacman -Qi`), проверяет их через `pacman -T` и предлагает переустановить недостающие с `--asdeps` (чтобы они не попали в world через `--regen-world-from-explicit`);
- `--revdep-rebuild` читает ELF-файлы (`DT_NEEDED`, `DT_RPATH`/`DT_RUNPATH`) и находит бинарники, линкующиеся с несуществующим `.so` (классическая поломка после обновления `openssl`/`icu`/`boost`). Пакеты AUR/локальные пересобираются через песочницу; пакеты официальных репозиториев пересобрать нельзя, поэтому недостающая библиотека ищется через `pacman -F`, а пакет, который её предоставляет, предлагается к установке. `--exclude` и [маска](#packagemask-и-packageunmask) действуют.

```bash
emerge -u --devel
emerge --check-devel
emerge @preserved-rebuild -p     # только список
emerge --revdep-rebuild -p       # только скан
emerge --revdep-rebuild          # скан, затем вопрос перед исправлением
```

### Удаление

`-C` удаляет указанные пакеты, `-c` — сирот, `--prune` — всё, чего нет в world. `-C` печатает, что `selected`, `protected` и `omitted`; затем `-C` и `-c` ждут 5 секунд (`Unmerging in: 5 4 3 2 1`, Ctrl-C прерывает), если только `-a` уже не спросил или не задан `-p`. После `-C` имена также убираются из world.

`-c` никогда не удаляет запись world, члена набора, записанного в world, запись `package.protect` и пакет `@system` из профиля. Он сообщает, сколько пропущено; `--show-protected` перечисляет их. С аргументами `-c` работает как зависимостно-осведомлённый unmerge: удаляются только пакеты, нужные никому (`-cv` показывает обратные зависимости, блокирующие удаление).

```bash
emerge -cp --show-protected
emerge -c
emerge --unmerge --ask linux-zen-headers
```

### Поиск и информация

`-s` ищет в официальных репозиториях, затем в ABS, затем в AUR. `-s aur/<name>` — только AUR, `-s abs/<name>` — ABS, `-s <repo>/<name>` — сначала этот репозиторий; `--repos` ограничивает поиск официальными репозиториями; `--search-all` показывает одно и то же имя из каждого репозитория. Замаскированные пакеты помечаются `[ Masked ]`. `-sv` добавляет установленную версию, а для AUR ещё и мейнтейнера, голоса, популярность, домашнюю страницу, дату обновления и лицензию.

`emerge --info` печатает версию (и версию libalpm), ядро, CPU/память/swap, действующие `--jobs`/`--jobsa`, каждый репозиторий в порядке `pacman.conf` с зеркалом, значения из `makepkg.conf` и `make.conf` (и какие перекрытия победили), `FEATURES`, профиль, размеры mask/world и итоги журнала merge.

`emerge --pkginfo nano` — карточка установленного пакета (как короткий `pacman -Qi`); с `-v` дополнительно перечисляет файлы.

### Сборка без установки

| Флаг | Что делает |
|------|------------|
| `-B`, `--buildpkgonly` | Только архивы, без установки: официальные => в `CacheDir`; AUR/ABS/локальные => собираются |
| `-f`, `--fetchonly` | Только загрузка всего дерева зависимостей: официальные => архивы в `CacheDir`; AUR/ABS => clone + исходники, проверенные в песочнице (`makepkg --verifysource`, в `SRCDEST`), для пакета и его AUR/ABS-зависимостей; официальные зависимости — архивами. Без сборки и установки |
| `--outdistfiles <DIR>` | С `-B`/`-f`: скопировать архивы (вместе с зависимостями) в DIR; checkout'ы AUR/ABS линкуются туда же |
| `-O`, `--nodeps` | Без разрешения зависимостей: официальные цели (`pacman -Sdd`); с `--usepkg` — один архив (`-Udd`). AUR/ABS: не реализовано, выводится предупреждение |
| `-o`, `--onlydeps` | Только зависимости официальных целей (не сами цели), неявно `-1`. AUR/ABS: не реализовано |

`-B` и `-f` не используют локальные бинарники из `ae-*`. Полученное можно развернуть позже: `emerge --usepkg <DIR>` ставит все архивы из каталога одной транзакцией, `emerge --usepkg -O <file>` — один архив без зависимостей.

### `--resume` и `--undo`

`--resume` продолжает прерванную транзакцию (`--skipfirst` пропускает первый пакет). `--undo` откатывает последнюю успешную операцию (один шаг истории):

- **после установки** — удаляет только пакеты, которые были в этом запуске новыми, и возвращает установленные пакеты, удалённые как блокеры;
- **после удаления** — переустанавливает из записанного источника (`Err/` и `abs/` только перечисляются);
- **после depclean** — переустанавливает удалённых сирот **как зависимости** (в world не добавляются): пакеты из репозиториев — из репозиториев, локальные сборки — через AUR, если он их ещё содержит. Версии записываются: если в репозиториях теперь другая, вам об этом скажут; пакет, которого нигде нет, перечисляется со строкой `pacman -U` для файла из кеша. Если часть не встала, в сохранённом состоянии остаются только они, и `--undo` можно запустить снова;
- **после `-u`** — не поддерживается (нет снимка версий).

### Журнал и новости

`/var/log/emerge.log` — только дозапись, одна строка на событие; сбой записи никогда не ломает merge. Пакеты AUR/ABS собираются по одному, поэтому у каждого свой таймер; `pacman -S`/`-Syu` — одна транзакция, поэтому её строка содержит все атомы и общее время. `--info` показывает итоги.

```
2026-09-21 08:51:44  MERGE    aur    noctalia-5.1.0-1.1        (12s)
2026-09-21 09:03:02  MERGE    repo   nano-8.0-1  vim-9.1-1     (4s)
2026-09-21 09:10:05  UNMERGE  -      old-package-2.0-1
```

`--log <PATH>` дополнительно пишет лог сессии (команда, строки emerge.log, полный вывод сборки) — для баг-репортов; только дозапись.

`emerge --news` получает [RSS-ленту новостей Arch Linux](https://archlinux.org/feeds/news/); состояние прочитанного — в `~/.cache/aura-emerge/news.state` (root не нужен).

```bash
emerge --news        # список, непрочитанные помечены [N]
emerge --news 3      # прочитать запись 3
emerge --news all    # пометить всё прочитанным
```

## Репозитории

### Локальные репозитории

С `FEATURES="localrepo"` каждый пакет, который aura-emerge собрал из AUR, ABS или локального PKGBUILD (или взял из архива `--usepkg`), после аудита и установки раскладывается в `/var/lib/aura-emerge/repos/<repo>/`, а база репозитория обновляется. Каталоги принадлежат root (`0755`). Это аналог aurutils: источники aur / abs / local разделены на уровне базы libalpm.

| Репозиторий | Что хранит |
|-------------|------------|
| `ae-aur` | сборки из AUR |
| `ae-abs` | сборки из ABS (`--abs`) |
| `ae-local` | `--usepkgbuild` и `--usepkg` |

Репозитории наполняются в любом случае, но pacman их **видит**, только если они указаны в `/etc/pacman.conf` *после* официальных (`core` / `extra` / `multilib`). Два способа:

```ini
# 1) подключить готовый файл (misc/ae-repos.conf => /etc/portage/ae-repos.conf)
Include = /etc/portage/ae-repos.conf
```

```ini
# 2) или записать секции самому
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

Что это даёт: `pacman -Qm` больше не показывает эти пакеты, работает `pacman -Si ae-aur/<pkg>`, а `pacman -S` / `-Su` могут переустановить их из проверенной копии. При совпадении имён побеждает первый репозиторий, где оно есть, поэтому секции `ae-*` должны стоять последними — официальные сохраняют приоритет. `SigLevel = Never` сделан намеренно: ничего здесь не подписано, пакеты проверены и закреплены sha256 самим emerge, а каталоги принадлежат root, так что подменить файл некому.

Что emerge делает дальше:

- **Повторное использование:** обычная установка пакета из AUR, версия которого в `ae-aur` совпадает с текущей в AUR, ставится из локального репозитория — без clone и сборки. `--edit` и `--pkgbuild-view` принудительно пересобирают; `-B` / `-f` локальные бинарники не используют; явный `ae-aur/<name>` идёт обычной установкой из репозитория.
- **Очистка:** `-C`, `-c` и `--prune` заодно убирают удалённое из `ae-*`. `emerge --localrepo-clean` убирает все записи, пакет которых уже не установлен (`-p` только показывает, `-a` спрашивает).
- **Пустые базы** создаются хуком `post_install` / `post_upgrade` пакета, командой `emerge --sync` и первой публикацией, так что `pacman -Sy` не спотыкается о перечисленный `file://`-сервер без пакетов.
- **Сбой публикации** — предупреждение, а не провал merge. Если секций `[ae-*]` нет, emerge один раз выводит подсказку со строкой `Include`.

### repos.conf

Список репозиториев в стиле Portage: `/etc/portage/repos.conf` (файл) или `/etc/portage/repos.conf/*.conf`. Включается через `FEATURES="repos"`. Тогда `emerge --info` показывает порядок по приоритету (по убыванию) и пишет `/etc/portage/ae-generated-repos.conf`. Подключите его в pacman **вместо** ручного перечисления тех же репозиториев: уберите соответствующие блоки `[repo]` из `pacman.conf` и добавьте

```ini
Include = /etc/portage/ae-generated-repos.conf
```

| Ключ | Содержание |
|------|------------|
| `priority` | Больше = выше приоритет. При равенстве — порядок объявления (только предупреждение, числа не переписываются) |
| `sync-uri` | Один или несколько URL (`$repo` / `$arch`) |
| `include` | Путь к mirrorlist pacman (`Server = …`); совместимо с rate-mirrors. Явные `sync-uri` идут первыми |
| `verify-signature` | Строка `SigLevel` как есть (`Never`, `Required DatabaseOptional`, …) |
| `location` | Необязательно (для `file://` — каталог репозитория) |

Шаблон в поставке — `misc/repos.conf` (секции закомментированы):

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

С `FEATURES="localrepo"` отсутствующие секции `ae-aur` / `ae-abs` / `ae-local` подставляются с приоритетами 10 / 9 / 8.

## Безопасность

### Привилегии

Фронтенд никогда не работает от root. Если запуск от root (`sudo emerge …`, `doas`, `pkexec`, `su -`), он сначала стартует root-helper, затем навсегда понижает права до вызвавшего пользователя (берётся из `AE_USER`, `SUDO_UID`, `DOAS_USER`, `PKEXEC_UID` или audit login uid; если ничего не найдено — остановка). От обычного пользователя helper стартует через `sudo` при первой необходимости. Резолвер, сборки AUR/ABS, песочница и все кеши работают без привилегий; вся работа с libalpm (install/remove/sync/upgrade/`-U`, проверка подписей) идёт через helper (`--ae-service`), который проверяет каждый запрос и сам применяет `package.mask` (`helper refused: … masked by …`).

**Условия запуска helper:** бинарник принадлежит root и не group/world-writable (`install -m755 -o root -g root`), uid = 0, stdin/stdout — пайпы (не TTY), родительский процесс жив. Конфликты чужих файлов на `-Su` / `-U` разрешаются через AE Force, а не срывают обновление.

### Сканирование PKGBUILD

Перед каждой установкой из AUR `PKGBUILD` (и любой `.install`) загружается из cgit и сканируется в два слоя:

1. **AST** (`tree-sitter-bash`): `curl|sh`, `base64`/`xxd | sh`, `source <(curl …)`, `eval "$(curl …)"`, `sudo`/`pkexec`/`doas`, inline-выполнение для python/perl/ruby/node/lua, `sh -c` с подозрительной нагрузкой, подстановка команд на верхнем уровне, обходы через конкатенацию строк и т. п.
2. **Строковые эвристики:** `chmod 777`, «голые» IPv4, сторонние менеджеры пакетов (`npm`/`pnpm`/`bun`/`yarn`/`pip`/`gem install …`, только удалённые `cargo install`/`go install`), `bash <(curl …)`, `curl … | python3/perl/node/…`, распаковка сразу в шелл (`gunzip`/`zstd -d`/`unzip -p`/`tar -xO | sh`), `/dev/tcp` и `/dev/udp`, heredoc'и, скармливаемые шеллу, paste-сайты, `.onion`, проверки на антипесочницу, известные IOC кампаний (Atomic Arch, openconnect-sso, `compromised.txt` 2018 года, …).

Находки бывают `Suspicious` или `ConfirmedIoc`. Срабатывание не блокирует жёстко: вы получаете файл и строку, ссылку на cgit и `Continue anyway? [y/N]`.

**Вспомогательные файлы.** Когда checkout лежит на диске (сборка или `--usepkgbuild <PATH> --scan`), дополнительно проверяются:

- `Makefile` (`Makefile`/`makefile`/`GNUmakefile`) и shell-скрипты, на которые ссылается PKGBUILD (`*.sh`, `*.bash`, `*.zsh`, `*.fish`) — построчно: `curl|sh`, base64/hex в shell, «голые» IP и остальные shell-IOC;
- патчи (`*.patch`, `*.diff`, `*.dpatch`, `*.pch` или содержимое, похожее на unified/git diff) — по заголовкам и добавленным строкам: пути вне дерева исходников (абсолютные, `..`, `.git`), создание symlink, бинарные hunk'и, слишком длинные добавленные строки, сетевые вызовы в build/script-целях и те же shell-IOC.

`--scan <pkg> --aur` видит только то, что отдаёт cgit (`PKGBUILD` и `.install`).

### Просмотр PKGBUILD и локальные источники

- **`--pkgbuild-view`** показывает PKGBUILD прямо перед сборкой и просит подтверждение: diff с последней показанной копией (кеш `~/.cache/aura-emerge/pkgbuild-view/`) или весь файл в первый раз. В случае отказа предлагает открыть `$EDITOR`, а не просто пропускает пакет. Работает после `--edit`, так что вы видите собственные правки. Применяется только к напрямую запрошенным пакетам AUR/ABS — не к рекурсивным зависимостям и не к пакетным путям (`@world`, `-u`, `--undo`).
- **`--usepkgbuild <PATH>`** собирает и устанавливает локальный каталог с PKGBUILD тем же конвейером, что AUR/ABS (сканер => песочница), а не голым `makepkg -si`. В `PATH` должен быть `PKGBUILD`; отсутствующий `.SRCINFO` создаётся. Сочетается с `--pkgbuild-view`, `-a`, `-1`, `--skippgp`, `--no-sandbox`, `--unshare-net-build`, `--skip-srcinfo-regen`, но не с пакетами/`@наборами` и `--aur`/`--abs`. В world записывается как `local/`, если нет `-1`.
- **`--usepkg <FILE…|DIR>`** устанавливает готовые архивы `.pkg.tar.*` (результат makepkg, пакет из файлового репозитория, каталог после `-f --outdistfiles`) без какого-либо PKGBUILD. Каждый архив фиксируется хешем, распаковывается и аудируется, повторно хешируется непосредственно перед привилегированной установкой и только затем передаётся libalpm (детали — в разделе «Проверка готовых архивов»). В world — `local/`, если нет `-1`. Несколько архивов за запуск — одна транзакция; зависимости между ними разрешаются вместе.
- **`--scan`** проверяет PKGBUILD (+ `.install`), ничего не собирая и не устанавливая: для AUR берёт файлы из cgit, для локального каталога — `--usepkgbuild <PATH> --scan`. Отчёт тот же, что перед сборкой, без вопроса «Continue anyway?». Ненулевой код при любой находке, так что годится для скриптов. Для `--abs` пока недоступен.

```bash
emerge neovim-git --aur --pkgbuild-view
emerge --usepkgbuild ~/src/my-pkg --pkgbuild-view --skippgp
emerge neovim-git --aur --scan
```

### Песочница

Сборки **AUR и ABS** выполняют `pkgver()` / `prepare()` / `build()` / `check()` / `package()` внутри [`bwrap`](https://github.com/containers/bubblewrap). Это обязательно; отказаться можно только явным `--no-sandbox`. Если `bubblewrap` не установлен, сборка отклоняется.

- пустой tmpfs поверх `/home` и `/run` (нет настоящего `$HOME`, сессионной D-Bus, ssh-agent, gpg-agent);
- `--clearenv` плюс небольшой белый список (`PATH`, `TERM`, `LANG`, каталоги назначения сборки, …);
- копия связки ключей GnuPG только с публичными ключами;
- PKGBUILD и `*.install` монтируются повторно только для чтения;
- запись только в каталог сборки (+ настроенные `PKGDEST` / `SRCDEST` / …);
- сеть по умолчанию общая, остальные пространства имён отключены;
- `fakeroot` в `package()` работает во вложенном `bwrap` с uid 0;
- seccomp-denylist (ptrace, kexec, загрузка/выгрузка модулей, mount/umount, pivot_root, reboot, swapon/swapoff, iopl/ioperm, bpf, userfaultfd, …).

| Флаг | Эффект |
|------|--------|
| `--no-sandbox` | Отключить bwrap; обычный `makepkg -si` (также пропускает подтверждение безаудитного fallback'а) |
| `--unshare-net-build` | Сеть только для загрузки и `prepare()`; `build()`/`check()`/`package()` — без сети |

Установка зависимостей (`pacman -S`) и финальный `pacman -U` выполняются **вне** песочницы — они не запускают код PKGBUILD.

### Проверка готовых архивов

Третий барьер перед установкой: готовый `*.pkg.tar.*` (результат сборки или `--usepkg`) проверяется через `bsdtar`:

- setuid/setgid-бинарники;
- скрипты `.INSTALL` (повторно сканируются сканером PKGBUILD);
- чувствительные пути: хуки alpm, юниты systemd, sudoers, udev, tmpfiles.d, binfmt.d, pam.d, dbus system.d, dispatcher.d в NetworkManager, environment.d, sysusers.d, `ld.so.preload` и т. п.;
- path traversal и symlink-имена членов архива.

На TTY нужно подтверждение даже без `--ask`; в неинтерактивном режиме — предупреждение и продолжение. Сбой аудита `bsdtar` останавливает установку, а не ведёт к установке без аудита.

### Единственный fallback с подтверждением

PKGBUILD, чьи зависимости нельзя разрешить статически (нет `.SRCINFO` и динамический `depends=(…)`), — единственный путь сборки, заканчивающийся голым `makepkg -si` без песочницы и аудита. Молча он не запускается: в интерактиве — явный вопрос Да/Нет для этого пакета, в `--noconfirm` — отказ (fail-closed). `--no-sandbox` этот вопрос пропускает: это сознательный отказ от песочницы на весь запуск.

### Прочее усиление

- Текст новостей Arch очищается (ANSI/bidi не попадают в терминал или `news.state`).
- Файлы конфига, масок, env и world открываются с `O_NOFOLLOW`, так что символическую ссылку нельзя подсунуть через гонку.
- Rust-зависимости проверяются через [`cargo deny`](https://github.com/EmbarkStudios/cargo-deny) (`make deny`).

<details>
<summary><h3>Проверка сканера (после установки)</h3></summary>

```bash
git clone https://github.com/Undercat037/ae-scanner-checker.git
emerge --usepkgbuild ./ae-scanner-checker --scan
```
</details>

<details>
<summary><h2>Флаги</h2></summary>

| Флаг | Описание |
|------|----------|
| `-s`, `--search` | Поиск пакетов |
| `-S`, `--searchdesc` | Поиск по описанию |
| `--search-all` | С `-s`: показать одно и то же имя из каждого репозитория (как `pacman -Ss`) |
| `-sv` | Подробная информация о пакете |
| `-v`, `--verbose` | Подробный вывод / детальная информация при поиске (`-sv`) |
| `-u`, `--update` | Полное обновление системы (world не читает) |
| `--devel` | С `-u`: также пересобрать `-git`/`-hg`/`-svn`/`-bzr`, чей upstream ушёл вперёд |
| `--check-devel` | Показать devel-пакеты с изменениями в upstream (без пересборки) |
| `-C`, `--unmerge` | Удалить пакеты (unmerge) |
| `-c`, `--depclean` | Удалить сирот (depclean) |
| `--show-protected` | С `-c`: показать сирот, оставленных только из-за записи в world |
| `-p`, `--pretend` | Пробный прогон (dry run) |
| `-a`, `--ask` | Спросить перед действием |
| `--sudoloop` | Держать sudo-метку «тёплой» (обновление каждые 60 с) весь запуск |
| `-1`, `--oneshot` | Установить как зависимость (в world не попадает) |
| `-n`, `--noreplace` | Не переустанавливать, если уже есть (та же или более новая версия) |
| `--with-optdeps` | Также установить опциональные зависимости пакетов из официальных репозиториев |
| `--aur` | Принудительно только AUR (зависимости только из AUR собираются рекурсивно всегда) |
| `--repos` | Только официальные репозитории (AUR не трогать) |
| `--abs` | Сборка из ABS (`pkgctl`) в песочнице bwrap |
| `--no-sandbox` | Отключить bwrap для AUR/ABS; обычный makepkg без изоляции (также пропускает подтверждение fallback'а) |
| `--unshare-net-build` | Без сети во время build()/check()/package() (в `prepare()` сеть остаётся) |
| `--skippgp` | Пропустить проверки PGP при сборке ABS (`--skippgpcheck`) |
| `--autopgp` | Автоматически импортировать недостающие ключи PGP перед сборкой `--abs` |
| `--edit` | Отредактировать PKGBUILD в `$EDITOR` перед сборкой (только запрошенный пакет); после сохранения пересоздаёт `.SRCINFO` |
| `--skip-srcinfo-regen` | С `--edit`: не пересоздавать `.SRCINFO` |
| `--pkgbuild-view` | Показать/сравнить PKGBUILD и подтвердить перед сборкой (только запрошенный пакет; при отказе предлагает `$EDITOR`) |
| `--usepkgbuild <PATH>` | Собрать и установить локальный каталог с PKGBUILD (сканер + bwrap); в world записывается как `local/` без `-1` |
| `--usepkg <FILE…\|DIR>` | Установить локальные архивы `.pkg.tar.*` через аудит + helper `-U` одной транзакцией (зависимости между ними разрешаются вместе; каталог = все архивы в нём, например результат `-f --outdistfiles`); в world записываются как `local/` без `-1` |
| `-O` / `--nodeps` | С `--usepkg`: установить ровно один файл-архив без проверки зависимостей (`pacman -Udd`) |
| `--scan` | Только аудит PKGBUILD/.install, без сборки/установки; ненулевой код при находках (AUR или `--usepkgbuild`) |
| `--sync` | Синхронизировать базы пакетов |
| `--refresh` | Принудительный `-Syy` (с `--sync` или `-u`) |
| `--info` | Информация о системе, как `emerge --info` в Gentoo |
| `--pkginfo <pkg…>` | Данные установленного пакета (`-v` перечисляет файлы) |
| `--news [N\|all]` | Новости Arch Linux: без аргумента — список, `N` — прочитать запись, `all` — пометить всё прочитанным |
| `--select` | Добавить в world без установки |
| `--deselect` | Убрать из world без удаления |
| `--check-world` | Показать записи world, которых нет в системе или которые установлены неявно |
| `--sync-explicit` | Пометить как зависимости явно установленные пакеты, которых нет в world, в `@set` из world и в `package.profile` (чтобы `-c` их видел). Самостоятельно: выполняется сразу (`-p` показывает, `-a` спрашивает). С установкой/обновлением: после успешного завершения. Можно в `EMERGE_DEFAULT_OPTS` |
| `--prune` | Удалить пакеты, которых нет в world |
| `--localrepo-clean` | Убрать из `ae-aur` / `ae-abs` / `ae-local` пакеты, которых уже нет в системе (`FEATURES="localrepo"`; `-p` показывает, `-a` спрашивает) |
| `--regen` | Пересоздать кеш метаданных пакетов |
| `--regen-world` | Заново определить префиксы репозиториев в world |
| `--regen-sets <SET>` | Заново определить префиксы в собственном наборе (`@game-kit` или `game-kit`) |
| `--regen-sort` | С `--regen-sets`: заодно отсортировать файл (комментарии и группировка теряются) |
| `--regen-world-from-explicit` | Миграция: добавить в world явно установленные пакеты (`pacman -Qeq`), которых нет в world, в `@set` из world и в `package.profile` (разово). `-p` только показывает, `-a` спрашивает |
| `--err-install` | С `@world`: также устанавливать записи `Err/` (источник неизвестен) |
| `--resume` | Продолжить прерванную транзакцию |
| `--skipfirst` | С `--resume`: пропустить первый пакет |
| `--undo` | Отменить последнюю установку/удаление/depclean (один шаг; `-u` не откатывается) |
| `--clean-source-cache` | Очистить постоянный кеш VCS-исходников и выйти |
| `--batchinstall <FILE>` | Массовая установка из текстового списка (формат как у собственного набора) |
| `--list-sets` | Список доступных `@наборов` (используется и автодополнением) |
| `--exclude <ATOM>` | Исключить пакет из этого запуска (повторяемый / через запятую) |
| `--keep-going` | Не останавливать пакетную операцию на первой ошибке; сбои попадают в итог в конце |
| `--revdep-rebuild` | Пересобрать пакеты, чьи бинарники линкуются с отсутствующей библиотекой |
| `-t`, `--tree` | Показать план деревом зависимостей |
| `-D`, `--deep[=N]` | С `-t`: вложенность на N уровней, без значения — на все |
| `--jobs <N>` | Максимум параллельных установок из официальных репозиториев (по умолчанию 1) |
| `--jobsa <N>` | Максимум параллельных сборок AUR/ABS (по умолчанию 1) |
| `--load-average <X>` | Не запускать новые сборки, пока нагрузка за минуту выше X |
| `--nospinner` | Без живой строки Jobs |
| `--color <y\|n\|auto>` | Принудительно включить или выключить цвет |
| `--quiet-build=n` | То же, что `--debug` (написание Gentoo) |
| `--list-profiles` | Список профилей в стиле Portage (как `eselect profile list`) |
| `--set-profile <N\|PATH>` | Установить `/etc/portage/make.profile`: индекс из `--list-profiles` или путь |
| `--aeforce` | AE Force: переносить мешающие установке файлы в `<file>.aeforce` без вопроса |
| `--noaefile` | AE Force: не оставлять файлов `.ae*` (старый сохранить, новый отбросить; с `--aeforce`: старый отбросить) |
| `--aelist` | Список файлов, оставленных AE Force (`.aenew` / `.aeforce` / `.aepark`) |
| `--aeclean [new\|old\|park\|all]` | Удалить файлы AE Force после вопроса (без аргумента = `all`) |
| `--ignore-default-opts` | Игнорировать `EMERGE_DEFAULT_OPTS` из make.conf на этот запуск |
| `--debug` | Показывать вывод makepkg/компилятора вживую (также `AE_DEBUG=1`); по умолчанию скрыт, чтобы строки `>>>` оставались читаемыми, а при сбое сборки сохранённый лог всё равно печатается |
| `--log <PATH>` | Записать лог сессии (команда, строки emerge.log, полный вывод сборки) для баг-репортов — `\| tee` не нужен; только дозапись: существующее содержимое никогда не обрезается и не перезаписывается |
| `-B` / `--buildpkgonly` | Только архивы, без установки (официальные => CacheDir; AUR/ABS/локальные => сборка) |
| `-f` / `--fetchonly` | Только загрузка всего дерева зависимостей: официальные => архивы в CacheDir (с зависимостями); AUR/ABS => clone + исходники, проверенные в песочнице (`makepkg --verifysource`, в SRCDEST), для пакета и его AUR/ABS-зависимостей, официальные зависимости — архивами. Без сборки и установки |
| `--outdistfiles <DIR>` | С `-B`/`-f`: скопировать архивы (вместе с зависимостями) в DIR; checkout'ы AUR/ABS линкуются туда же |
| `-N`/`--newuse`, `-e`/`--emptytree`, `-q`, `-o`, `-l`, `-A`, `--onlydeps`, `--complete-graph`, `--changed-use`, `--backtrack`, `--columns`, `--newrepo`, `--reinstall`, `--with-bdeps`, `--metadata`, `--clean`, `--config`, `--noconfmem`, `--verbose-conflicts` | Совместимость с Gentoo: принимаются, ничего не делают |
| `-h`, `--help` | Справка |
| `-V`, `--version` | Версия |

Man-страница `emerge(1)` генерируется из CLI (`emerge --gen-manpage`), поэтому не может разойтись с `--help`.
</details>

## Автор / Лицензия
Copyright (C) 2026 **Undercat037**
Лицензия GPL-3.0
