# aura-emerge
en | [ua](UA-README.MD)

> A standalone Gentoo-style `emerge` package manager for Arch Linux - installs from official repos, the AUR, and ABS; scans PKGBUILDs for supply-chain attack patterns before building; and runs untrusted build steps inside a `bwrap` sandbox.

Packages are tracked in a `world.set` file - install once, track forever.

## Installation

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
<summary><h3>Manual/makepkg (Unstable)</h3></summary>

### Manual build
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

**Dependencies:** `git`  
**Optional:**
- `devtools` - `--abs` support (`pkgctl repo clone`)
- `gnupg` - automatic PGP key import (`--autopgp`)
- `bubblewrap` - build sandbox for AUR/ABS (without it, falls back to plain unisolated `makepkg`)

## Gentoo → Arch: make.conf equivalent

Arch has no USE flags, but compilation can be tuned the same way as [Gentoo](https://wiki.gentoo.org/wiki/Main_Page)'s `/etc/portage/make.conf` via `/etc/makepkg.conf`:

```bash
sudo nvim /etc/makepkg.conf
```

| Gentoo (`make.conf`)         | Arch (`makepkg.conf`)         | Purpose                      |
|------------------------------|-------------------------------|------------------------------|
| `CFLAGS="-O2 -march=native"` | `CFLAGS="-O2 -march=native"`  | Compiler optimisation flags  |
| `CXXFLAGS="-O2 -march=..."`  | `CXXFLAGS="-O2 -march=..."`   | C++ compiler flags           |
| `RUSTFLAGS="-C opt-level=3"` | `RUSTFLAGS="-C opt-level=3"`  | Rust compiler flags          |
| `MAKEOPTS="-j$(nproc)"`      | `MAKEFLAGS="-j$(nproc)"`      | Parallel build jobs          |
| `USE="lto"`                  | `OPTIONS+=(lto)`              | Link-time optimisation       |
| `FEATURES="ccache"`          | `BUILDENV+=(ccache)`          | Build cache                  |
| `PORTDIR` / `PKGDIR`         | `PKGDEST`, `SRCDEST`          | Build artefact directories   |

See [GCC optimization flags](https://gcc.gnu.org/onlinedocs/gcc/Optimize-Options.html) and [rustc codegen options](https://doc.rust-lang.org/rustc/codegen-options/index.html).

[**→ Example makepkg.conf**](repo-files/makepkg.conf)

## Usage

```bash
emerge neovim                              # official repos, falls back to AUR
emerge neovim-git --aur                    # AUR only (AUR-only deps built recursively)
emerge foo-git --aur --sudoloop            # same, but keep sudo warm through the whole recursive build
emerge neovim --only-repos                 # official repos only, never touch AUR
emerge neovim --abs                        # build from ABS (pkgctl), in bwrap sandbox
emerge neovim --abs --no-sandbox           # ABS without sandbox (plain makepkg -si)
emerge neovim --abs --skippgp              # ABS, skip PGP signature checks
emerge neovim --abs --autopgp              # ABS, auto-import missing PGP keys
emerge neovim --abs --edit                 # ABS, edit PKGBUILD ($EDITOR), regenerates .SRCINFO
emerge neovim-git --aur --edit             # AUR, edit PKGBUILD, regenerates .SRCINFO
emerge neovim-git --aur --edit --skip-srcinfo-regen  # edit without regenerating .SRCINFO
emerge neovim-git --aur --unshare-net-build     # no network during build()/check()/package()
emerge -c                                  # remove orphans (depclean)
emerge -C neovim                           # remove package (unmerge)
emerge @world                              # provision: install missing entries from world.set
emerge @world --err-install                   # also retry Err/ (unknown-source) entries normally
emerge -u @world                           # full system upgrade (repos + AUR); does not read world.set
emerge -u @world --refresh                 # upgrade with forced -Syy first
emerge --sync                              # sync package databases
emerge @game-kit                           # custom set from /etc/emerge/sets.d/game-kit.set
emerge @game-kit neovim                    # sets + plain packages
emerge --list-sets                         # list available @sets
emerge --info                              # system info (Gentoo-style emerge --info)
emerge --news                              # Arch Linux news, flags unread
emerge --news 3                            # read item 3 in full
emerge --news all                          # mark all as read
emerge neovim-git --aur --edit --pkgbuild-view  # view/diff the PKGBUILD too, on top of --edit
emerge neovim --abs --pkgbuild-view        # same for an ABS build
emerge --install-pkgbuild ~/src/my-pkg        # build+install a local PKGBUILD checkout (scanner + bwrap, like AUR/ABS)
emerge --install-pkgbuild ~/src/my-pkg --pkgbuild-view --skippgp
emerge --scan neovim-git --aur              # audit only, no build
emerge --install-pkgbuild ~/src/my-pkg --scan  # same, for a local checkout
emerge -u --devel                           # full upgrade + upstream check for -git/-hg/-svn/-bzr
emerge --check-devel                        # just report which devel packages moved upstream
emerge -s neovim                           # search (repos + AUR)
emerge -s neovim --only-repos              # search official repos only
emerge --searchdesc neovim                 # search by description
emerge -sv neovim                          # detailed package info
emerge -sv neovim-git --aur                # detailed info from AUR only
emerge -1 htop                             # install as dependency (not added to world.set)
emerge -n neovim                           # do not reinstall if present (--noreplace)
emerge -p neovim                           # dry run
emerge -a neovim                           # ask before acting
emerge --select neovim                     # add to world.set without installing
emerge --deselect neovim                   # remove from world.set without unmerging
emerge --prune                             # remove packages not in world.set
emerge --regen                             # regenerate package metadata cache
emerge --regen-world                       # re-resolve repo prefixes in world.set
emerge --regen-sets @game-kit              # re-resolve prefixes for one custom set
emerge --regen-sets @game-kit --regen-sort # same + alphabetically sort the file
emerge --regen-world-from-explicit         # one-time migration from pacman -Qeq
emerge --resume                            # resume interrupted transaction
emerge --resume --skipfirst                # resume, skip first package
emerge --undo                              # undo last install/unmerge
emerge --clean-source-cache                # wipe persistent VCS source cache (-git/-hg/-svn)
emerge @preserved-rebuild                  # check installed packages for missing deps
emerge @preserved-rebuild -p               # pretend: list only
emerge @preserved-rebuild -a               # ask before each pacman prompt
emerge --batchinstall pkgs.txt              # mass-install from a list file
emerge --batchinstall pkgs.txt --aur -a     # combine with any other flag
```

## Sandbox

**AUR and ABS** builds run the PKGBUILD functions `pkgver()` / `prepare()` / `build()` / `check()` / `package()` inside a [`bwrap`](https://github.com/containers/bubblewrap) namespace (when `bubblewrap` is installed):

- fake `$HOME` (no SSH keys, GPG secret keyring, browser profile);
- writes only inside the build directory (plus configured `PKGDEST` / `SRCDEST` / `SRCPKGDEST` / `BUILDDIR`);
- network shared by default (`--share-net`);
- other namespaces (user / IPC / PID / UTS / cgroup) torn down.

This is a **second, independent layer** on top of the [PKGBUILD scanner](#security-pkgbuild-scanning): the scanner blocks *known* bad patterns before anything runs; the sandbox contains whatever it missed.

Dependency installation (`pacman -S`) and the final `pacman -U` stay **outside** the sandbox - neither executes PKGBUILD-authored code.

| Flag | Effect |
|------|--------|
| `--no-sandbox` | Disable bwrap; plain `makepkg -si` |
| `--unshare-net-build` | Download/`prepare()` keep network; `build()`/`check()`/`package()` run with network off |

Without `bubblewrap` or with `--no-sandbox`, the build runs unisolated (with a warning).

## Source cache (SRCDEST)

For `-git` / `-hg` / `-svn` packages, aura-emerge defaults to a persistent `SRCDEST` under `~/.cache/aura-emerge/sources` (or `$XDG_CACHE_HOME/aura-emerge/sources`). Makepkg then does an incremental `git fetch` instead of a full re-clone every run.

```bash
emerge --clean-source-cache   # delete that cache
```

If `makepkg.conf` already sets `SRCDEST`, that value is used instead.

## Batch install

`--batchinstall <FILE>` reads a plain-text package list and folds it into the normal package list before anything runs - same format as a [custom set](#custom-sets) (one atom per line, `#` comments allowed), but from any path you give directly, no need to register it under `/etc/emerge/sets.d/` first.

```
# pkgs.txt
neovim
aur/visual-studio-code-bin
firefox
```

```bash
emerge --batchinstall pkgs.txt
```

Since it's folded in alongside `cli.packages`, it combines with every other flag exactly like typed-out names would (`-p`, `-a`, `--aur`, `--abs`, `--only-repos`, ...).

## PKGBUILD review

`--pkgbuild-view` shows the PKGBUILD about to be built right before the build starts, and asks for confirmation - a diff against the last-shown copy of that pkgbase when one's cached under `~/.cache/aura-emerge/pkgbuild-view/`, the full file the first time. Declining offers to open it in `$EDITOR` right there instead of just skipping the package. Combines with `--edit` (which runs first, so the view/diff reflects whatever you just edited). Applies to `--aur`/plain AUR-fallback installs and `--abs`, always only for a directly-requested package - never for a recursively-pulled-in AUR dependency, and never for a batch path (`@world` provisioning, `-u`, `--undo`).

```bash
emerge neovim-git --aur --pkgbuild-view
emerge neovim --abs --edit --pkgbuild-view   # edit first, then view/diff the result
```

`--install-pkgbuild <PATH>` builds and installs an arbitrary local PKGBUILD checkout through the same pipeline as an AUR/ABS install - scanner, then the bwrap sandbox - instead of a bare, unaudited `makepkg -si`. `PATH` must contain a `PKGBUILD`; if there's no `.SRCINFO` yet, one is generated (`makepkg --printsrcinfo`, same as `--edit` does) so dependencies resolve statically. Combines with `--pkgbuild-view`, `-a`/`--ask`, `-1`/`--oneshot`, `--skippgp`, `--no-sandbox`, `--unshare-net-build`, `--skip-srcinfo-regen`; not compatible with any package/`@set` argument or `--aur`/`--abs`. Recorded in `world.set` with the `Err/` prefix (source unknown - see [world.set](#worldset)) unless `-1`/`--oneshot` is also given.

```bash
emerge --install-pkgbuild ~/src/my-pkg
emerge --install-pkgbuild ~/src/my-pkg --pkgbuild-view --skippgp
```

`--scan` audits a PKGBUILD (+ `.install`) without building or installing anything - fetches straight from cgit for an AUR package name, or reads a local checkout directly with `--install-pkgbuild <PATH> --scan`. Same findings/report as the normal pre-build scan, minus the "Continue anyway?" gate (nothing to continue to). Exits non-zero on any finding, so it's usable in a script. Not yet wired up for `--abs`.

```bash
emerge --scan neovim-git --aur
emerge --install-pkgbuild ~/src/my-pkg --scan
```

## Devel package upstream check

Arch's `-git`/`-hg`/`-svn`/`-bzr` convention has `pkgver()` compute the version at build time from the VCS history - but the version *recorded on the AUR page itself* only moves when the maintainer manually re-pushes it, which for an actively-developed package can lag real upstream activity by weeks or months. Plain `-u` only ever compares that recorded AUR version, so it routinely never flags these for a rebuild even when upstream has moved on.

`--devel` (with `-u`) and `--check-devel` (standalone, report-only) close that gap: for every installed devel package, they parse the `git+` source URL straight out of its PKGBUILD and run `git ls-remote` against it directly - no local clone needed - comparing the result against the hash last seen (`~/.cache/aura-emerge/devel.state`). git-only for now; `-hg`/`-svn`/`-bzr` packages are recognized but reported as "couldn't determine" rather than silently skipped.

```bash
emerge -u --devel        # normal full upgrade, plus rebuild any devel package whose upstream moved
emerge --check-devel     # just report which ones moved, no rebuild
```

## @preserved-rebuild

Arch has no preserved-libs tracking. `@preserved-rebuild` collects every installed package’s declared dependencies (`pacman -Qi`), feeds the deduplicated list to `pacman -T`, and offers to reinstall unsatisfied ones via `pacman -S --asdeps` (so they don’t look “explicit” and get vacuumed into world.set by a later `--regen-world-from-explicit`).

## world.set

Path: `/etc/emerge/world.set`. Every explicitly installed package is added automatically and removed on unmerge. Editable by hand (one package per line).

```
cachyos-extra-v3/rust-analyzer
chaotic-aur/visual-studio-code-bin
core/nano
aur/ayugram-desktop-bin
abs/nano
extra/firefox
```

Prefixes: official repo name, `aur/`, `abs/`, `Err/` (source unknown).

> **On `Err/`:** used both for a genuinely local build with no traceable origin and for an `abs/`/`aur/` entry that later became unresolvable. `--regen-world` / `--regen-sets` preserve a known `abs/`/`aur/` prefix instead of collapsing it to `Err/` on the first re-resolution miss.

Batch installs no longer abort the whole request if one name is missing - the rest still install, and missing names are reported at the end with a non-zero exit code.

### `@world` has two meanings

- **`emerge @world` (no `-u`)** - *provision*. Installs whatever `world.set` lists that isn’t already on the system; already-installed packages are left alone. Prefix decides the source. Keep `world.set` in dotfiles, clone on a fresh machine, run `emerge @world`.
- **`emerge -u @world`** (or `emerge -u`) - *upgrade*. Upgrades everything already installed (repos + AUR). Does not read `world.set`.
- **`emerge --sync`** - database refresh only (`-Sy` / `-Syy`).

## Custom sets

`/etc/emerge/sets.d/<name>.set` is invoked as `@<name>`:

```
# /etc/emerge/sets.d/game-kit.set
steam
lutris
gamemode
aur/protonup-qt
```

One package per line, `#` comments allowed. Sets combine freely:

```bash
emerge @game-kit @dev-tools neovim
```

`emerge --list-sets` and shell completion (`emerge @<TAB>`) list available sets.

## --undo

Reverses the last successful install or unmerge (one step of history):

- **after install** - removes only packages that were brand-new in that run;
- **after unmerge** - reinstalls from the recorded source (`Err/`/`abs/` are listed only);
- **after `-u`** - not supported (no version snapshot).

```bash
emerge --undo
```

## News

`emerge --news` fetches the official [Arch Linux news RSS feed](https://archlinux.org/feeds/news/). Read state lives in `~/.cache/aura-emerge/news.state` (no root needed).

```bash
emerge --news        # list, unread marked [N]
emerge --news 3      # read item 3
emerge --news all    # mark all as read
```

## Security: PKGBUILD scanning

Before every AUR install, the raw `PKGBUILD` (and any `.install` hook) is fetched from cgit and scanned in two layers:

1. **AST** (`tree-sitter-bash`) - `curl|sh`, `base64`/`xxd | sh`, `source <(curl …)`, `eval "$(curl …)"`, `sudo`/`pkexec`/`doas`, python inline-exec, top-level command substitution (`` `...` `` / `$(...)` outside function bodies - e.g. leftover markdown backticks in `pkgdesc`), etc. Trivial text evasions (string concatenation, absolute paths, `env` wrappers) do not slip through.
2. **Line heuristics** - `chmod 777`, raw IPv4 literals, foreign package managers (`npm install …`), paste sites, `.onion` addresses, known IOCs (Atomic Arch, openconnect-sso, 2018 `compromised.txt`, …).

Findings are `Suspicious` or `ConfirmedIoc`. A hit does not hard-block - it prints the file/line, a cgit link, and asks `Continue anyway? [y/N]`.

AUR and ABS builds are additionally constrained by the [bwrap sandbox](#sandbox).

<details>
<summary><h3>Testing(post-install)</h3></summary>

```bash
git clone https://github.com/Undercat037/aura-emerge.git
cd aura-emerge
emerge --install-pkgbuild ./scaner-test --ask
```
</details>


<details>
<summary><h2>Flags</h2></summary>

| Flag | Description |
|------|-------------|
| `-s` | Search packages |
| `--searchdesc` | Search by description |
| `-sv` | Detailed package info |
| `-u` | Full system upgrade (does not read world.set) |
| `--devel` | With `-u` - also rebuild `-git`/`-hg`/`-svn`/`-bzr` packages whose upstream moved |
| `--check-devel` | Report which devel packages have upstream changes (no rebuild) |
| `-C` | Remove packages (unmerge) |
| `-c` | Remove orphans (depclean) |
| `-p` | Pretend (dry run) |
| `-a` | Ask before acting |
| `--sudoloop` | Keep sudo timestamp warm for the whole run |
| `-1` | Install as dependency (oneshot) |
| `-v` | Verbose output |
| `-n` | Do not reinstall if present (noreplace) |
| `--aur` | Force AUR (AUR-only dependencies are always resolved and built recursively) |
| `--only-repos` | Official repos only |
| `--abs` | Build from ABS (`pkgctl`), in bwrap sandbox |
| `--no-sandbox` | Disable bwrap for AUR/ABS |
| `--unshare-net-build` | No network during build()/check()/package() |
| `--skippgp` | Skip PGP checks (ABS) |
| `--autopgp` | Auto-import missing PGP keys (ABS) |
| `--edit` | Edit PKGBUILD before building; regenerates `.SRCINFO` after save |
| `--pkgbuild-view` | Show/diff PKGBUILD, confirm before building (offers `$EDITOR` on decline) |
| `--install-pkgbuild <PATH>` | Build+install a local PKGBUILD checkout (scanner + bwrap) |
| `--scan` | Audit-only PKGBUILD/.install scan, no build/install (AUR or `--install-pkgbuild`) |
| `--skip-srcinfo-regen` | With `--edit` - do not regenerate `.SRCINFO` |
| `--sync` | Sync package databases |
| `--refresh` | Force `-Syy` (with `--sync` or `-u`) |
| `--info` | System info |
| `--news [N\|all]` | Arch Linux news |
| `--check-news [N\|all]` | Alias for `--news` |
| `--select` | Add to world.set without installing |
| `--deselect` | Remove from world.set without unmerging |
| `--prune` | Remove packages not in world.set |
| `--regen` | Regenerate metadata cache |
| `--regen-world` | Re-resolve repo prefixes in world.set |
| `--regen-sets <SET>` | Re-resolve prefixes for one custom set |
| `--regen-sort` | With `--regen-sets` - also sort the file |
| `--regen-world-from-explicit` | Migration: seed world.set from `pacman -Qeq` |
| `--err-install` | On `@world`, also retry `Err/` entries |
| `--resume` | Resume interrupted transaction |
| `--skipfirst` | Skip first package on resume |
| `--undo` | Undo last install/unmerge |
| `--clean-source-cache` | Wipe persistent VCS source cache |
| `--batchinstall <FILE>` | Mass-install from a package list file |
| `--list-sets` | List available `@sets` |
| `-h` / `--help` | Help |
| `-V` / `--version` | Version |

The `emerge(1)` man page is generated from the CLI via `clap_mangen` (`emerge --gen-manpage`), so it cannot drift from `--help`.
</details>

## Author/License
Copyright (C) 2026 **Undercat037**
GPL-3.0 license