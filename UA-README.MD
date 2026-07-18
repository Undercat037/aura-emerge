# aura-emerge
[en](README.MD) | ua

> Обгортка у стилі Gentoo `emerge` для Arch Linux, побудована на основі [Aura](https://github.com/fosskers/aura).

Керує пакунками через файл `world.set` — встановив один раз, відстежується назавжди.

## Встановлення

```bash
git clone https://github.com/Undercat037/aura-emerge
cd aura-emerge
makepkg -si
```
або через AUR:
```bash
paru -S aura-emerge 
```
[Релізний бінарник](target/release/)

**Вимоги:** `aura` має бути встановлена та доступна в `/usr/bin/aura`.  
**Опційно:** `asp` для підтримки `--abs` (`emerge asp`), `gpg` для автоматичного імпорту PGP-ключів (`--autopgp`).

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
emerge neovim --only-repos         # встановити лише з офіційних репо, без звернення до AUR
emerge neovim --abs                # зібрати з ABS (Arch Build System) через asp
emerge neovim --abs --skippgp      # зібрати з ABS, пропустити перевірку PGP підпису
emerge neovim --abs --autopgp      # зібрати з ABS, автоімпорт відсутніх PGP-ключів, без запиту
emerge neovim --abs --edit         # зібрати з ABS, редагувати PKGBUILD перед збіркою ($EDITOR)
emerge neovim-git --aur --edit     # встановити з AUR, редагувати PKGBUILD перед збіркою (aura --hotedit)
emerge -c                          # видалити сиріт (depclean)
emerge -C neovim                   # видалити конкретний пакунок (unmerge)
emerge -u @world                   # оновити всю систему (офіційні + AUR)
emerge -u @world --refresh         # оновити, примусово освіжити бази перед цим (як pacman -Syy)
emerge --sync                      # синхронізувати бази даних
emerge --info                      # показати інформацію про систему (як Gentoo emerge --info)
emerge -s neovim                   # пошук в офіційних репо + AUR
emerge -s neovim --only-repos      # пошук лише в офіційних репо, без AUR
emerge --searchdesc neovim         # пошук за описом пакунка
emerge -sv neovim                  # детальна інформація (aura -Si / -Ai)
emerge -sv neovim-git --aur        # детальна інформація виключно з AUR
emerge -1 aura                     # встановити як залежність (не додається у world.set)
emerge -n neovim                   # не перевстановлювати, якщо вже встановлено (--noreplace)
emerge -p neovim                   # показати, що буде зроблено (dry run)
emerge -a neovim                   # запитати перед встановленням
emerge --select neovim             # додати до world.set без встановлення
emerge --deselect neovim           # прибрати з world.set без видалення
emerge --prune                     # видалити пакунки, яких немає у world.set
emerge --regen                     # оновити кеш метаданих пакунків
emerge --regen-world               # оновити префікси репозиторіїв у world.set
emerge --resume                    # продовжити перерваний процес
emerge --resume --skipfirst        # продовжити, пропустивши перший пакунок
emerge @preserved-rebuild          # перевірити встановлені пакунки на відсутні залежності, запропонувати перевстановити
emerge @preserved-rebuild -p       # pretend: лише вивести список відсутніх залежностей
emerge @preserved-rebuild -a       # запитувати перед кожним запитом pacman під час перевстановлення
```

## @preserved-rebuild

В Arch немає відстеження preserved-libs, тому `@preserved-rebuild` збирає всі залежності оголошені кожним встановленим пакунком (`pacman -Qi`), передає дедуплікований список у `pacman -T` (який коректно розв'язує provides/віртуальні пакунки), і пропонує перевстановити те, що виявиться незадоволеним, через `aura -S`. 

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

Префікси: назва офіційного репозиторію, `aur/` для AUR, `abs/` для зібраних з ABS локально, `Err/` якщо джерело не вдалося визначити.

## Безпека: сканування PKGBUILD

Перед кожним встановленням з AUR (`--aur`, звичайний fallback в AUR та `-u @world`) aura-emerge завантажує «сирий» `PKGBUILD` (і будь-який пов'язаний `.install`-хук) з cgit-дзеркала AUR і перевіряє його на типові підозрілі патерни, перш ніж передати керування `aura`:

- `curl | sh` / `wget -qO- | bash` — пересилання виводу одразу в шелл
- виконання коду через base64-декодування, hex-екрановані payload'и
- `chmod 777`, «сирі» IPv4-адреси прямо в скриптах
- звернення до стороннього пакетного менеджера (`npm install`, `pip install` тощо) всередині збірки
- назви пакунків, що збігаються з відомими індикаторами компрометації з розкритих атак на ланцюг постачання AUR (наприклад, інцидент «Atomic Arch»)

Спрацювання не блокує встановлення повністю — виводиться файл/рядок, що спрацював, посилання на перегляд джерела на cgit, і запит `Continue anyway? [y/N]` перед продовженням. Це дешева «розтяжка», а не заміна самостійного перегляду PKGBUILD.

## Прапори

| Прапор | Опис |
|--------|------|
| `-s` | Пошук пакета |
| `--searchdesc` | Пошук за описом |
| `-sv` | Детальна інформація про пакет |
| `-u` | Оновити систему (@world) |
| `-C` | Видалити конкретні пакети (unmerge) |
| `-c` | Видалити сиріт (depclean) |
| `-p` | Симуляція (dry run) |
| `-a` | Запит підтвердження |
| `-1` | Встановити як залежність (oneshot) |
| `-v` | Детальний вивід (verbose) |
| `-n` | Не перевстановлювати (noreplace) |
| `--aur` | Примусово використати AUR |
| `--only-repos` | Лише офіційні репо, без пошуку/встановлення з AUR |
| `--abs` | Зібрати з ABS через asp |
| `--skippgp` | Пропустити перевірку PGP при збірці з ABS |
| `--autopgp` | Автоматично імпортувати відсутні PGP-ключі з довіреного keyserver при збірці з ABS, без запиту |
| `--edit` | Редагування PKGBUILD перед збіркою — відкриває `$EDITOR` для `--abs`, або передає `--hotedit` в `aura` для AUR |
| `--sync` | Синхронізувати бази даних |
| `--refresh` | Примусово освіжити всі бази, навіть якщо вони актуальні (`pacman -Syy`), використовується разом з `--sync` або `-u` |
| `--info` | Показати інформацію про систему, як `emerge --info` в Gentoo |
| `--select` | Додати до world.set (без встановлення) |
| `--deselect` | Прибрати з world.set (без видалення) |
| `--prune` | Видалити пакунки поза world.set |
| `--regen` | Оновити кеш метаданих |
| `--regen-world` | Оновити префікси репо у world.set |
| `--resume` | Продовжити перерваний процес |
| `--skipfirst` | Пропустити перший пакунок при resume |
| `--with-bdeps` | Підтягнути залежності збірки (сумісність) |
| `--verbose-conflicts` | Детальні конфлікти слотів (сумісність) |
| `-h` / `--help` | Показати довідку |
| `-V` / `--version` | Показати версію |

## Демонстрація

[vhs-emerge](repo-files/emerge.gif)

## Автор

**Undercat037**