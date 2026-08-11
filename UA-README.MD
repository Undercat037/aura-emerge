# aura-emerge
[en](README.MD) | ua

> Автономний, у стилі Gentoo `emerge` для Arch Linux - встановлює з офіційних репозиторіїв, AUR та ABS, сканує PKGBUILD на ознаки атак на ланцюг постачання перед збіркою, і виконує недовірені кроки збірки всередині пісочниці `bwrap`.

Керує пакунками через файл `world.set` — встановив один раз, відстежується назавжди.

## Встановлення

### Makepkg
```bash
git clone https://github.com/Undercat037/aura-emerge
cd aura-emerge
makepkg -si
```
### AUR:
```bash
paru -S aura-emerge
```
### Ручна збірка:

```bash
git clone https://github.com/Undercat037/aura-emerge
cd aura-emerge
cargo build --release 
sudo cp -r target/release/aura-emerge /usr/bin/emerge
ln -s /usr/bin/emerge /usr/bin/portageq
emerge --gen-completions [your-shell] # /usr/share/bash-completion/completions/emerge , /usr/share/zsh/site-functions/_emerge , /usr/share/fish/vendor_completions.d/emerge.fish
emerge --gen-manpage > /usr/share/man/man1/emerge.1
```

**Вимоги:** `git` та `curl` (git-клони AUR, RPC/cgit-запити, стрічка новин).  
**Опційно:** `devtools` для підтримки `--abs` (`emerge devtools`, надає `pkgctl repo clone`), `gnupg` для автоматичного імпорту PGP-ключів (`--autopgp`), `bubblewrap` для пісочниці при збірці (див. [Пісочниця](#пісочниця) — за відсутності відбувається відкат до звичайної незахищеної збірки).

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

Довідка по `CFLAGS`/`CXXFLAGS` — [прапори оптимізації GCC](https://gcc.gnu.org/onlinedocs/gcc/Optimize-Options.html), по `RUSTFLAGS` — [параметри кодогенерації rustc](https://doc.rust-lang.org/rustc/codegen-options/index.html).

[**→ Приклад makepkg.conf**](repo-files/makepkg.conf)

## Використання

```bash
emerge neovim                      # встановити пакунок (офіційні репо, автоматичний пошук в AUR)
emerge neovim-git --aur            # встановити виключно з AUR
emerge foo-git --aur --aur-deep    # також рекурсивно зібрати AUR-only залежності (синонім --aur-full)
emerge neovim --only-repos         # встановити лише з офіційних репо, без звернення до AUR
emerge neovim --abs                # зібрати з ABS через pkgctl repo clone, за замовчуванням у пісочниці bwrap
emerge neovim --abs --no-sandbox   # зібрати з ABS без пісочниці (звичайний makepkg -si, без ізоляції bwrap)
emerge neovim --abs --skippgp      # зібрати з ABS, пропустити перевірку PGP підпису
emerge neovim --abs --autopgp      # зібрати з ABS, автоімпорт відсутніх PGP-ключів, без запиту
emerge neovim --abs --edit         # зібрати з ABS, редагувати PKGBUILD перед збіркою ($EDITOR), автоматично оновлює .SRCINFO
emerge neovim-git --aur --edit     # встановити з AUR, редагувати PKGBUILD перед збіркою, автоматично оновлює .SRCINFO
emerge neovim --aur --edit --off-src-regen  # редагувати PKGBUILD без подальшого оновлення .SRCINFO
emerge -c                          # видалити сиріт (depclean)
emerge -C neovim                   # видалити конкретний пакунок (unmerge)
emerge @world                      # provision: встановити відсутнє з world.set (нова машина тощо)
emerge @world --err-inst           # provision, з повторною спробою для записів Err/ (нез'ясоване джерело) через звичайний шлях встановлення
emerge -u @world                   # повне оновлення системи (офіційні + AUR) — world.set не читається
emerge -u @world --refresh         # повне оновлення, примусово освіжити бази перед цим (як pacman -Syy)
emerge --sync                      # синхронізувати бази даних
emerge @game-kit                   # встановити власний набір з /etc/emerge/sets.d/game-kit.set
emerge @game-kit neovim            # набори можна поєднувати зі звичайними пакунками
emerge --list-sets                 # показати @world, @preserved-rebuild та всі власні набори
emerge --info                      # показати інформацію про систему (як Gentoo emerge --info)
emerge --news                      # показати новини Arch Linux, найновіші спочатку, позначає непрочитані
emerge --news 3                    # прочитати новину 3 повністю, позначає прочитаною
emerge --news all                  # позначити всі новини як прочитані
emerge -s neovim                   # пошук в офіційних репо + AUR
emerge -s neovim --only-repos      # пошук лише в офіційних репо, без AUR
emerge --searchdesc neovim         # пошук за описом пакунка
emerge -sv neovim                  # детальна інформація
emerge -sv neovim-git --aur        # детальна інформація виключно з AUR
emerge -1 htop                     # встановити як залежність (не додається у world.set)
emerge -n neovim                   # не перевстановлювати, якщо вже встановлено (--noreplace)
emerge -p neovim                   # показати, що буде зроблено (dry run)
emerge -a neovim                   # запитати перед встановленням
emerge --select neovim             # додати до world.set без встановлення
emerge --deselect neovim           # прибрати з world.set без видалення
emerge --prune                     # видалити пакунки, яких немає у world.set
emerge --regen                     # оновити кеш метаданих пакунків
emerge --regen-world               # оновити префікси репозиторіїв у world.set
emerge --regen-sets @game-kit      # оновити префікси репозиторіїв для одного власного набору
emerge --regen-world-from-explicit # одноразова міграція: наповнити world.set з усіх явно встановлених пакунків
emerge --resume                    # продовжити перерваний процес
emerge --resume --skipfirst        # продовжити, пропустивши перший пакунок
emerge --undo                      # скасувати останнє встановлення або видалення
emerge --clean-source-cache        # видалити постійний кеш VCS-джерел (-git/-hg/-svn чекаути)
emerge @preserved-rebuild          # перевірити встановлені пакунки на відсутні залежності, запропонувати перевстановити
emerge @preserved-rebuild -p       # pretend: лише вивести список відсутніх залежностей
emerge @preserved-rebuild -a       # запитувати перед кожним запитом pacman під час перевстановлення
```

## Пісочниця

Збірки `--abs` виконують функції PKGBUILD `pkgver()`/`prepare()`/`build()`/`check()`/`package()` всередині простору імен [`bwrap`](https://github.com/containers/bubblewrap), увімкненого за замовчуванням при наявності `bubblewrap`: фейковий `$HOME` (без SSH/GPG/браузера), нічого не пишеться поза директорією збірки (+ налаштовані `PKGDEST`/`SRCDEST` тощо), мережа спільна, решта просторів імен (user/IPC/PID/UTS/cgroup) знищена.

Другий, незалежний рівень поверх [сканера PKGBUILD](#безпека-сканування-pkgbuild) — сканер блокує *відомі* погані патерни до запуску, пісочниця обмежує наслідки того, що він пропустив. Встановлення залежностей і фінальний `pacman -U` лишаються поза пісочницею (там не виконується код з PKGBUILD).

`--no-sandbox` вимикає пісочницю на користь звичайного `makepkg -si`. Збірки з AUR поки без пісочниці — ізоляція bwrap наразі покриває лише `--abs`.

## @preserved-rebuild

В Arch немає відстеження preserved-libs, тому `@preserved-rebuild` збирає залежності кожного встановленого пакунка (`pacman -Qi`), передає дедуплікований список у `pacman -T`, і пропонує перевстановити незадоволені через `pacman -S --asdeps`. `--asdeps` важливий — інакше вони виглядали б «явно встановленими» і потрапили б у world.set при наступному `--regen-world-from-explicit`.

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

Префікси: назва офіційного репозиторію, `aur/` для пакунків з AUR, `abs/` для зібраних локально з ABS, `Err/` якщо джерело неможливо визначити.

> **Про `Err/`:** цей префікс використовується як для справді локально зібраного пакунка без відстежуваного походження, так і для `abs/`/`aur/`-пакунка, що згодом став нерозпізнаваним (наприклад, видалений з AUR) — щойно запис звівся до `Err/`, ця відмінність втрачається назавжди. `--regen-world`/`--regen-sets` тепер зберігають відомий префікс `abs/`/`aur/` під час regen замість того, щоб понижувати його до `Err/` при першій же невдалій спробі повторного розв'язання — тож це стосується лише записів, які вже були `Err/` до цього виправлення.

Встановлення кількох пакунків одночасно більше не перериває весь пакет команд, якщо один з них ніде не знайдено (одруківка, видалений з AUR тощо) — решта все одно встановлюються, а відсутні назви виводяться в кінці з ненульовим кодом виходу.

### `@world` має два різних значення

Це найважливіша поведінка у стилі Gentoo, тож варто пояснити детально:

- **`emerge @world` (без `-u`)** — *provision*. Встановлює те, чого немає з `world.set`; встановлене раніше не зачіпається. Префікс визначає джерело (офіційний репо, AUR-резолвер зі скануванням PKGBUILD, або `abs`/нерозпізнане — лише виводиться, потрібен `--abs`/ручне встановлення). Операція «перенести world.set на нову машину»: тримайте у dotfiles, клонуйте, `emerge @world`. Явно вказані пакунки встановляться першими, напр. `emerge nano @world`.
- **`emerge -u @world`** (або `emerge -u`) — *upgrade*. Оновлює все встановлене, репо + AUR. `world.set` не чіпає.
- **`emerge --sync`** — лише освіжує бази (`-Sy`/`-Syy`).

## Власні набори (sets)

Окрім `@world` та `@preserved-rebuild`, можна визначати власні набори у `/etc/emerge/sets.d/`. Файл `/etc/emerge/sets.d/game-kit.set` викликається як `@game-kit`:

```
# /etc/emerge/sets.d/game-kit.set
steam
lutris
gamemode
aur/protonup-qt
```

Один пакунок на рядок, порожні рядки та `#`-коментарі ігноруються. Набори можна поєднувати між собою і зі звичайними пакунками в одній команді:

```bash
emerge @game-kit @dev-tools neovim
```

`emerge --list-sets` виводить усі доступні набори (`@world`, `@preserved-rebuild`, а також по одному рядку на кожен файл `sets.d/*.set`) — саме це викликає й автодоповнення в шеллі, тож `emerge @<TAB>` пропонує реальні назви наборів.

## --undo

Скасовує останнє успішне встановлення або видалення. Зберігається лише один крок історії, на найкращих зусиллях:

- **Після встановлення:** видаляє лише пакунки, нові в цьому запуску (`pacman -R`, прибирає з world.set). Оновлення/перевстановлення в тому ж запуску не чіпаються.
- **Після видалення:** перевстановлює за записаним джерелом кожного атому (`Err/`/`abs/` лише виводяться у списку). Не відновить точно ту саму версію, якщо вона вже не актуальна.
- **Після `-u`:** не підтримується — немає ані посторінкової історії версій, ані власного механізму знімків. Відкатуйте вручну через `pacman -Qi`/лог/кеш pacman.

```bash
emerge --undo
```

## Новини

`emerge --news` завантажує офіційний [RSS-фід новин Arch Linux](https://archlinux.org/feeds/news/) і показує список останніх записів, найновіші спочатку, позначаючи непрочитані як `[N]` — та сама ідея, що й `eselect news` у Gentoo, оскільки в Arch немає кроку `Sync`, під час якого зазвичай спливають подібні новини про breaking changes. Стан прочитаності зберігається окремо для кожного користувача в `~/.cache/aura-emerge/news.state`, тож для цього не потрібен root.

```bash
emerge --news        # список останніх новин, позначає непрочитані
emerge --news 3      # показати новину 3 повністю, позначити прочитаною
emerge --news all    # позначити всі поточні новини як прочитані
```

## Безпека: сканування PKGBUILD

Перед кожним встановленням з AUR aura-emerge завантажує «сирий» `PKGBUILD` (і `.install`-хук) з cgit-дзеркала і сканує його перед збіркою у два шари:

- **AST-перевірки** (`tree-sitter-bash`) для найважливіших сигналів — RCE-конвеєрів, `sudo`/`pkexec`/`doas` посеред збірки, `eval` над curl/wget, python inline-exec — розбираються як справжнє синтаксичне дерево, тож тривіальні текстові обходи не проходять.
- **Текстові евристики** як резерв/доповнення: `curl|sh`, base64/hex/openssl-декодовані payload'и, `chmod 777`, «сирі» IP, звернення до сторонніх пакетних менеджерів (`npm install`, `pip install`…), paste-сайти, `.onion`-адреси, та відомі IOC (назви/хеші) з розкритих кампаній (напр. «Atomic Arch» червень 2026, хвиля `openconnect-sso` лип./серп. 2026, маркер `compromised.txt` з 2018 року).

Знахідки позначаються як `Suspicious` (правдоподібно, але типово) або `ConfirmedIoc` (точний збіг з відомою кампанією, показується гучніше).

Спрацювання не блокує встановлення — виводиться файл/рядок, посилання на cgit, запит `Continue anyway? [y/N]`. Дешева «розтяжка», не заміна самостійного перегляду. Збірки `--abs` додатково захищені [пісочницею `bwrap`](#пісочниця) як другим, незалежним рівнем.

## Прапори

| Прапор | Опис |
|--------|------|
| `-s` | Пошук пакета |
| `--searchdesc` | Пошук за описом |
| `-sv` | Детальна інформація про пакет |
| `-u` | Повне оновлення системи (world.set не читається; див. розділ world.set) |
| `-C` | Видалити конкретні пакети (unmerge) |
| `-c` | Видалити сиріт (depclean) |
| `-p` | Симуляція (dry run) |
| `-a` | Запит підтвердження |
| `-1` | Встановити як залежність (oneshot) |
| `-v` | Детальний вивід (verbose) |
| `-n` | Не перевстановлювати (noreplace) |
| `--aur` | Примусово використати AUR |
| `--aur-deep` (синонім `--aur-full`) | Також рекурсивно розв'язати і зібрати AUR-only залежності; неявно вмикає `--aur` |
| `--only-repos` | Лише офіційні репо, без пошуку/встановлення з AUR |
| `--abs` | Зібрати з ABS через `pkgctl repo clone`, за замовчуванням у пісочниці `bwrap` |
| `--no-sandbox` | Вимкнути пісочницю `bwrap` для збірок `--abs`, звичайний `makepkg -si` |
| `--skippgp` | Пропустити перевірку PGP при збірці з ABS |
| `--autopgp` | Автоматично імпортувати відсутні PGP-ключі з довіреного keyserver при збірці з ABS, без запиту |
| `--edit` | Редагування PKGBUILD перед збіркою — відкриває `$EDITOR`, потім автоматично оновлює `.SRCINFO` за результатом редагування |
| `--off-src-regen` | Разом з `--edit` — не оновлювати `.SRCINFO` автоматично (стара поведінка) |
| `--sync` | Синхронізувати бази даних |
| `--refresh` | Примусово освіжити всі бази, навіть якщо вони актуальні (`pacman -Syy`), використовується разом з `--sync` або `-u` |
| `--info` | Показати інформацію про систему, як `emerge --info` в Gentoo |
| `--news [N\|all]` | Показати новини Arch Linux (без аргументу — список), прочитати новину N повністю, або `all` щоб позначити всі як прочитані |
| `--check-news [N\|all]` | Повний синонім для `--news` |
| `--select` | Додати до world.set (без встановлення) |
| `--deselect` | Прибрати з world.set (без видалення) |
| `--prune` | Видалити пакунки поза world.set |
| `--regen` | Оновити кеш метаданих |
| `--regen-world` | Оновити префікси репо у world.set |
| `--regen-sets <SET>` | Оновити префікси репо для одного власного набору |
| `--regen-sort` | Разом з `--regen-sets` — також відсортувати файл набору за абеткою |
| `--regen-world-from-explicit` | Одноразова міграція: наповнити world.set з усіх явно встановлених пакунків |
| `--err-inst` | При provisioning `@world`, також повторно спробувати записи `Err/` (нез'ясоване джерело) звичайним шляхом встановлення |
| `--resume` | Продовжити перерваний процес |
| `--skipfirst` | Пропустити перший пакунок при resume |
| `--undo` | Скасувати останнє встановлення або видалення (не повне оновлення системи — див. розділ `--undo`) |
| `--clean-source-cache` | Видалити постійний кеш VCS-джерел aura-emerge (`-git`/`-hg`/`-svn` чекаути) |
| `--list-sets` | Показати доступні `@world` / `@preserved-rebuild` / власні набори |
| `--with-bdeps` | Підтягнути залежності збірки (сумісність) |
| `--verbose-conflicts` | Детальні конфлікти слотів (сумісність) |
| `-h` / `--help` | Показати довідку |
| `-V` / `--version` | Показати версію |

Встановлюється man-сторінка `emerge(1)` — `man emerge` для повного довідника. Вона генерується безпосередньо з визначення CLI через [`clap_mangen`](https://github.com/clap-rs/clap/tree/master/clap_mangen) під час пакування (`emerge --gen-manpage`, за тим самим принципом, що й `--gen-completions`), тож не може розійтися з `--help`.

## Демонстрація

[vhs-emerge](repo-files/emerge.gif)

## Автор

**Undercat037**