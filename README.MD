# aura-emerge
en | [ua](UA-README.MD)

> A Gentoo-style `emerge` wrapper for Arch Linux, built on top of [Aura](https://github.com/fosskers/aura).

Manages packages via a `world.set` file — install once, track forever.

## Installation

```bash
git clone https://github.com/Undercat037/aura-emerge
cd aura-emerge
makepkg -si
```
or via AUR:
```bash
paru -S aura-emerge 
```
[Release binary](target/release/)

**Requirements:** `aura` must be installed and in `/usr/bin/aura`.  
**Optional:** `asp` for `--abs` support (`emerge asp`), `gpg` for automatic PGP key import (`--autopgp`).

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
emerge neovim                      # install package (official repos, fallbacks to AUR)
emerge neovim-git --aur            # install from AUR explicitly
emerge neovim --only-repos         # install only from official repos, never touch the AUR
emerge neovim --abs                # build from ABS (Arch Build System) source via asp
emerge neovim --abs --skippgp      # build from ABS, skip PGP signature check
emerge neovim --abs --autopgp      # build from ABS, auto-import missing PGP keys, no prompt
emerge neovim --abs --edit         # build from ABS, edit PKGBUILD before building ($EDITOR)
emerge neovim-git --aur --edit     # install from AUR, edit PKGBUILD before building (aura --hotedit)
emerge -c                          # remove orphans (depclean)
emerge -C neovim                   # remove specific package (unmerge)
emerge -u @world                   # update entire system (official + AUR)
emerge -u @world --refresh         # update, force-refresh databases first (like pacman -Syy)
emerge --sync                      # sync package databases
emerge --info                      # show system info (like Gentoo's emerge --info)
emerge -s neovim                   # search in official repos + AUR
emerge -s neovim --only-repos      # search official repos only, skip AUR
emerge --searchdesc neovim         # search by package description
emerge -sv neovim                  # detailed info (aura -Si / -Ai)
emerge -sv neovim-git --aur        # detailed info from AUR only
emerge -1 aura                     # install as dependency (no world.set)
emerge -n neovim                   # do not reinstall if already installed (--noreplace)
emerge -p neovim                   # pretend (dry run)
emerge -a neovim                   # ask before installing
emerge --select neovim             # add to world.set without installing
emerge --deselect neovim           # remove from world.set without unmerging
emerge --prune                     # remove packages not tracked in world.set
emerge --regen                     # regenerate package metadata cache
emerge --regen-world               # re-resolve repository prefixes in world.set
emerge --resume                    # resume last interrupted transaction
emerge --resume --skipfirst        # resume, skipping the first package
emerge @preserved-rebuild          # check installed packages for missing deps, offer to reinstall
emerge @preserved-rebuild -p       # pretend: just list missing dependencies
emerge @preserved-rebuild -a       # ask before each pacman prompt during reinstall
```

## @preserved-rebuild

There is no preserved-libs tracking in Arch, so `@preserved-rebuild` collects all dependencies declared by each installed package (`pacman -Qi`), passes the deduplicated list to `pacman -T` (which correctly resolves provides/virtual packages), and offers to reinstall any that are found to be unsatisfied via `aura -S`.

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

## Security: PKGBUILD scanning

Before every AUR install (`--aur`, plain fallback-to-AUR installs, and `-u @world`), aura-emerge fetches the raw `PKGBUILD` (and any referenced `.install` hook) from the AUR cgit mirror and greps it for common red-flag patterns before handing off to `aura`:

- `curl | sh` / `wget -qO- | bash` style pipe-to-shell
- base64-decoded execution, hex-escaped payloads
- `chmod 777`, raw IPv4 literals in scripts
- calls out to a foreign package manager (`npm install`, `pip install`, …) from inside the build
- package names matching known IOCs from disclosed AUR supply-chain campaigns (e.g. the "Atomic Arch" incident)

A hit doesn't block the install outright — it prints the offending file/line plus a direct link to review the source on cgit, and asks `Continue anyway? [y/N]` before proceeding. This is a cheap tripwire, not a substitute for actually reading the PKGBUILD yourself.

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
| `--only-repos` | Only official repos, never search/install from AUR |
| `--abs` | Build from ABS source via asp |
| `--skippgp` | Skip PGP checks when building from ABS |
| `--autopgp` | Auto-import missing PGP keys from a trusted keyserver when building from ABS, without prompting |
| `--edit` | Edit PKGBUILD before building — opens `$EDITOR` for `--abs`, or passes `--hotedit` to `aura` for AUR |
| `--sync` | Sync databases |
| `--refresh` | Force-refresh all databases even if up to date (`pacman -Syy`), used with `--sync` or `-u` |
| `--info` | Show system info, mirroring Gentoo's `emerge --info` |
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