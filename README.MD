# aura-emerge
en | [ua](UA-README.MD)
A Gentoo-style `emerge` wrapper for Arch Linux, built on top of [Aura](https://github.com/fosskers/aura).

Manages packages via a `world.set` file — install once, track forever.

## Installation

```bash
git clone https://github.com/Undercat037/aura-emerge
cd aura-emerge
cargo build --release
sudo cp target/release/aura-emerge /usr/local/bin/emerge
sudo mkdir -p /etc/emerge
sudo touch /etc/emerge/world.set
```

**Requirements:** `aura` must be installed and in `$PATH`.

## Usage

```bash
emerge nano                  # install package (official repos)
emerge nano --aur            # install from AUR explicitly
emerge -c                    # remove orphans (depclean)
emerge -u @world             # update entire system (official + AUR)
emerge --sync                # sync package databases
emerge -s nano               # search in official repos + AUR
emerge -sv nano              # detailed info (aura -Si / -Ai)
emerge -sv nano --aur        # detailed info from AUR only
emerge -1 nano               # install as dependency (no world.set)
emerge -p nano               # pretend (dry run)
emerge -a nano               # ask before installing
```

## world.set

Tracked at `/etc/emerge/world.set`. Each installed package is added automatically. You can edit it manually — one package per line.

```
firefox
nano
neovim
setrixtui
```

## Flags

| Flag | Description |
|------|-------------|
| `-s` | Search packages |
| `-sv` | Detailed package info |
| `-u` | Update |
| `-c` | Remove orphans |
| `-p` | Pretend (dry run) |
| `-a` | Ask for confirmation |
| `-1` | Install as dep (oneshot) |
| `-v` | Verbose |
| `--aur` | Force AUR |
| `--sync` | Sync databases |

## Author

**Undercat037**
