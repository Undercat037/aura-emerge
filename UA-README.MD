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
або через AUR:
```bash
paru -S aura-emerge 
```
[Релізний бінарник](target/release/)

**Вимоги:** `aura` має бути встановлена та доступна в `$PATH`.

## Використання

```bash
emerge nano                  # встановити пакунок (офіційні репо, автоматичний пошук в AUR)
emerge nano --aur            # встановити виключно з AUR
emerge -c                    # видалити сиріт (depclean)
emerge -C nano               # видалити конкретний пакунок (unmerge)
emerge -u @world             # оновити всю систему (офіційні + AUR)
emerge --sync                # синхронізувати бази даних
emerge -s nano               # пошук в офіційних репо + AUR
emerge -sv nano              # детальна інформація (aura -Si / -Ai)
emerge -sv nano --aur        # детальна інформація виключно з AUR
emerge -1 nano               # встановити як залежність (не додається у world.set)
emerge -O nano               # не перевстановлювати, якщо вже встановлено (--noreplace)
emerge -p nano               # показати, що буде зроблено (dry run)
emerge -a nano               # запитати перед встановленням
```

## world.set

Файл відстеження знаходиться за шляхом `/etc/emerge/world.set`. Кожен встановлений пакунок додається туди автоматично, а при видаленні — прибирається. Ви можете редагувати його власноруч (один пакунок на рядок).

```
firefox
nano
neovim
setrixtui
```

## Прапори

| Прапор | Опис |
|--------|------|
| `-s` | Пошук пакета |
| `-sv` | Детальна інформація про пакет |
| `-u` | Оновити систему (@world) |
| `-C` | Видалити конкретні пакети (unmerge) |
| `-c` | Видалити сиріт (depclean) |
| `-p` | Симуляція (dry run) |
| `-a` | Запит підтвердження |
| `-1` | Встановити як залежність (oneshot) |
| `-v` | Детальний вивід (verbose)|
| `-O` | Не перевстановлювати (noreplace) |
| `--aur` | Примусово використати AUR |
| `--sync` | Синхронізувати бази даних |

## Автор

**Undercat037**
