# aura-emerge
[en](README.MD) | ua

> Обгортка у стилі Gentoo `emerge` для Arch Linux, побудована на основі [Aura](https://github.com/fosskers/aura).

Керує пакунками через файл `world.set` — встановив один раз, відстежується назавжди.

## Встановлення

```bash
git clone https://github.com/Undercat037/aura-emerge
cd aura-emerge
cargo build --release
sudo cp target/release/aura-emerge /usr/local/bin/emerge
sudo mkdir -p /etc/emerge
sudo touch /etc/emerge/world.set
```
або
```bash
paru -S aura-emerge 
```

**Вимоги:** `aura` має бути встановлена та доступна в `$PATH`.

## Використання

```bash
emerge nano                  # встановити пакунок (офіційні репозиторії)
emerge nano --aur            # встановити з AUR явно
emerge -c                    # видалити осиротілі пакунки (depclean)
emerge -u @world             # оновити всю систему (офіційні + AUR)
emerge --sync                # синхронізувати бази даних пакунків
emerge -s nano               # пошук в офіційних репо + AUR
emerge -sv nano              # детальна інформація (aura -Si / -Ai)
emerge -sv nano --aur        # детальна інформація лише з AUR
emerge -1 nano               # встановити як залежність (без world.set)
emerge -p nano               # симуляція (dry run)
emerge -a nano               # запитати підтвердження перед встановленням
```

## world.set

Зберігається у `/etc/emerge/world.set`. Кожен встановлений пакунок додається автоматично. Можна редагувати вручну — один пакунок на рядок.

```
firefox
nano
neovim
setrixtui
```

## Прапори

| Прапор | Опис |
|--------|------|
| `-s` | Пошук пакунків |
| `-sv` | Детальна інформація про пакунок |
| `-u` | Оновлення |
| `-c` | Видалення осиротілих пакунків |
| `-p` | Симуляція (dry run) |
| `-a` | Запит підтвердження |
| `-1` | Встановити як залежність (oneshot) |
| `-v` | Детальний вивід |
| `--aur` | Примусово використати AUR |
| `--sync` | Синхронізувати бази даних |

## Автор

**Undercat037**
