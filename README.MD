# aura-emerge
en | [ua](repo-files/UA-README.MD)

> A Gentoo-style `emerge` wrapper for Arch Linux, built on top of [Aura](https://github.com/fosskers/aura).

Manages packages via a `world.set` file — install once, track forever.

## Installation

```bash
git clone https://github.com/Undercat037/aura-emerge
cd aura-emerge
cargo build --release
sudo cp target/release/aura-emerge /usr/local/bin/emerge
sudo ln -sf /usr/local/bin/emerge /usr/local/bin/portageq
sudo mkdir -p /etc/emerge
sudo touch /etc/emerge/world.set
```
or via AUR:
```bash
paru -S aura-emerge 
```
[Release binary](target/release/)

**Requirements:** `aura` must be installed and in `/usr/bin/aura`.  
**Optional:** `asp` for `--abs` support (`emerge asp`).

## Gentoo → Arch: make.conf equivalent

Arch Linux doesn't have USE flags, but you can configure compilation options similarly to [Gentoo](https://wiki.gentoo.org/wiki/Main_Page)'s `/etc/portage/make.conf` by editing `/etc/makepkg.conf`:

```bash
sudo nvim /etc/makepkg.conf
```

Key options that map to Gentoo concepts:

| Gentoo (`make.conf`)         | Arch (`makepkg.conf`)         | Purpose                        |
|------------------------------|-------------------------------|--------------------------------|
| `CFLAGS="-O2 -march=native"` | `CFLAGS="-O2 -march=native"`  | Compiler optimisation flags    |
| `CXXFLAGS="-O2 -march=..."`  | `CXXFLAGS="-O2 -march=..."`   | C++ compiler flags             |
| `RUSTFLAGS="-C opt-level=3"` | `RUSTFLAGS="-C opt-level=3"`  | Rust compiler flags            |
| `MAKEOPTS="-j$(nproc)"`      | `MAKEFLAGS="-j$(nproc)"`      | Parallel build jobs            |
| `USE="lto"`                  | `OPTIONS+=(lto)`              | Link-time optimisation         |
| `FEATURES="ccache"`          | `BUILDENV+=(ccache)`          | Build cache                    |
| `PORTDIR` / `PKGDIR`         | `PKGDEST`, `SRCDEST`          | Build artefact directories     |

For `CFLAGS`/`CXXFLAGS` reference see [GCC optimization flags](https://gcc.gnu.org/onlinedocs/gcc/Optimize-Options.html), for `RUSTFLAGS` see [rustc codegen options](https://doc.rust-lang.org/rustc/codegen-options/index.html).

[**→ Example makepkg.conf**](repo-files/makepkg.conf)

## Usage

```bash
emerge nano                  # install package (official repos, fallbacks to AUR)
emerge nano --aur            # install from AUR explicitly
emerge nano --abs            # build from ABS (Arch Build System) source via asp
emerge nano --abs --skippgp  # build from ABS, skip PGP signature check
emerge nano --abs --edit     # build from ABS, with edit PKGBUILD
emerge -c                    # remove orphans (depclean)
emerge -C nano               # remove specific package (unmerge)
emerge -u @world             # update entire system (official + AUR)
emerge --sync                # sync package databases
emerge -s nano               # search in official repos + AUR
emerge --searchdesc nano     # search by package description
emerge -sv nano              # detailed info (aura -Si / -Ai)
emerge -sv nano --aur        # detailed info from AUR only
emerge -1 nano               # install as dependency (no world.set)
emerge -n nano               # do not reinstall if already installed (--noreplace)
emerge -p nano               # pretend (dry run)
emerge -a nano               # ask before installing
emerge --select nano         # add to world.set without installing
emerge --deselect nano       # remove from world.set without unmerging
emerge --prune               # remove packages not tracked in world.set
emerge --regen               # regenerate package metadata cache
emerge --regen-world         # re-resolve repository prefixes in world.set
emerge --resume              # resume last interrupted transaction
emerge --resume --skipfirst  # resume, skipping the first package
```

## world.set

Tracked at `/etc/emerge/world.set`. Each installed package is added automatically and removed upon unmerge. You can edit it manually — one package per line.

Packages are stored with their repository prefix for full traceability:

```
cachyos-extra-v3/rust-analyzer
chaotic-aur/visual-studio-code-bin
core/nano
aur/ayugram-desktop-bin
abs/nano-custom
extra/firefox
```

Prefixes: official repo name, `aur/` for AUR packages, `abs/` for locally built from ABS, `Err/` if the source cannot be determined.

## Flags

| Flag | Description |
|------|-------------|
| `-s` | Search packages |
| `--searchdesc` | Search by description |
| `-sv` | Detailed package info |
| `-u` | Update system (@world) |
| `-C` | Remove specific packages (unmerge) |
| `-c` | Remove orphans (depclean) |
| `-p` | Pretend (dry run) |
| `-a` | Ask for confirmation |
| `-1` | Install as dep (oneshot) |
| `-v` | Verbose output |
| `-n` | Do not reinstall (noreplace) |
| `--aur` | Force AUR |
| `--abs` | Build from ABS source via asp |
| `--skippgp` | Skip PGP checks when building from ABS |
| `--edit` | Edit PKGBUILD when building from ABS |
| `--sync` | Sync databases |
| `--select` | Add to world.set (no install) |
| `--deselect` | Remove from world.set (no unmerge) |
| `--prune` | Remove packages not in world.set |
| `--regen` | Regenerate metadata cache |
| `--regen-world` | Re-resolve repo prefixes in world.set |
| `--resume` | Resume interrupted transaction |
| `--skipfirst` | Skip first pkg when resuming |
| `--with-bdeps` | Pull in build-time deps (compat) |
| `--verbose-conflicts` | Verbose slot conflicts (compat) |
| `-h` / `--help` | Show help |
| `-V` / `--version` | Show version |

## Demo

[vhs-emerge](repo-files/emerge.gif)

## Author

**Undercat037**