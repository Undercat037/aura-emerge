<div align="center">
  <img src="misc/logo/slash-logo-128x128.svg" width="128" alt="aura-emerge logo">

# Aura-Emerge Secure Package Manager

en | [ua](UA-README.MD)
</div>

> A standalone Gentoo-style `emerge` package manager for Arch Linux - installs from official repos, the AUR, and ABS; scans PKGBUILDs for supply-chain attack patterns before building; and runs untrusted build steps inside a `bwrap` sandbox.

Packages are tracked in a `world` file - install once, track forever.

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

### Make
```bash
git clone https://github.com/Undercat037/aura-emerge.git
cd aura-emerge
make install
```
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

[**→ Example makepkg.conf**](misc/makepkg.conf)

## make.conf

`/etc/portage/make.conf` (and `~/.config/emerge/make.conf` for per-user overrides) is aura-emerge's own config file, separate from `makepkg.conf` - the Gentoo `make.conf` split between `EMERGE_DEFAULT_OPTS` and `CFLAGS` in one file. System file first, then the user one; last file to set a key wins.

```bash
sudo nvim /etc/portage/make.conf
```

```toml
# Flags spliced into every invocation, before whatever's typed on the
# command line (which always wins on a conflict). Disable for one run
# with --ignore-default-opts.
EMERGE_DEFAULT_OPTS = ["--pkgbuild-view", "--unshare-net-build"]

# Build flags, applied via a generated makepkg.conf (makepkg --config) -
# override /etc/makepkg.conf without touching it.
[build]
CFLAGS = "-march=native -O2 -pipe"
CXXFLAGS = "$CFLAGS"
RUSTFLAGS = "-C target-cpu=native"
MAKEFLAGS = "-j$(nproc)"
NINJAFLAGS = "-j$(nproc)"
OPTIONS = ["strip", "docs", "!debug", "lto"]
BUILDENV = ["!distcc", "color", "check", "!sign"]
```

Recognized keys: `EMERGE_DEFAULT_OPTS` at the top level, and in `[build]`: `CFLAGS`, `CXXFLAGS`, `CPPFLAGS`, `LDFLAGS`, `RUSTFLAGS`, `MAKEFLAGS`, `NINJAFLAGS`, `BUILDENV`, `OPTIONS`. Build keys are also accepted at the top level, outside `[build]`.

Scalars and lists are interchangeable everywhere - a list is joined with spaces, a string is split on them. Values are shell-expanded when makepkg sources the generated config, so `-j$(nproc)` works, and so does a reference to another key set above it, like `CXXFLAGS = "$CFLAGS"`; a TOML literal string (`'\$FOO'`) keeps a dollar sign literal.

`EMERGE_DEFAULT_OPTS` only accepts modifier flags (an allowlist: `--ask`, `--verbose`, `--devel`, `--tree`, `--searchdesc`, `--exclude=<atom>`, and similar - see the [example file](misc/make.conf)) - it can't turn every run into `--unmerge`. Conflicting pairs (`--aur`/`--abs`, `--aur`/`--only-repos`, `--no-sandbox`/`--unshare-net-build`, `--skippgp`/`--autopgp`, `--select`/`--deselect`) are rejected if both end up set, whether typed or from the config; a config default that conflicts with something typed on the command line is dropped instead, since the command line is the more specific intent. `--abs` and `--only-repos` are **not** a conflict - ABS builds an official-repo package from its own source, so the two agree.

`--info` shows what was read and the effective, shell-expanded value of each override.

[**→ Example make.conf**](misc/make.conf)

## Mask (/etc/portage/package.mask)

Portage's `package.mask`, for Arch atoms: packages this machine never installs, checked even as a transitive AUR dependency and even when requested by name. One entry per line in `/etc/portage/package.mask` or `/etc/portage/package.mask/*.mask`, `*` glob allowed, optional repo prefix, optional trailing `# reason`:

```
ayugram-desktop-bin            # specific package, any source
aur/*-bin                      # never install AUR -bin packages
*-git                          # no devel packages on this box
extra/nano                     # specific package from a specific repo
```

A masked package hard-stops the request - not a prompt, no override flag. Separate from `--exclude` below: a mask is a standing, persistent decision; `--exclude` is scoped to one run.

## --exclude / --keep-going

`--exclude <atom>` (repeatable, or comma-separated) leaves a package out of the current run - like `--exclude` in Portage. Applies to installs, `-u` (via `pacman --ignore`), `@world` provisioning, `--depclean`/`--prune` (protects from removal), and `--revdep-rebuild`.

```bash
emerge -u --exclude linux --exclude aur/ayugram-desktop-bin   # upgrade everything except these
emerge --exclude firefox @world                               # provision, skip firefox even if missing
```

`--keep-going` doesn't stop a batch (`@world`, `-u`, `--batchinstall`, multi-package installs) at the first failure - it keeps going, collects what failed, and prints a summary at the end with a non-zero exit. `--resume` after a `--keep-going` run retries only the packages that failed, not the whole batch.

```bash
emerge -u --keep-going                # full upgrade, don't stop on one bad AUR package
emerge --batchinstall pkgs.txt --keep-going
emerge --resume                       # retries just what failed last time
```

## -t/--tree, --deep

`-t`/`--tree` shows the plan (with `-p`/`--pretend`, or right before a real install) as a dependency tree instead of a flat list, `emerge -pt`-style. One level of nesting by default - what was typed at depth 0, whatever it pulled in as a dependency indented under it - attributed by cross-referencing `pacman -Si`'s "Depends On" for each requested package against everything else in the plan. Add `--deep` to nest through however many levels the actual dependency chain goes, or `--deep=N` to cap it at N levels, instead of stopping at direct deps.

```bash
emerge -apt noctalia --aur     # ask + pretend + tree: preview before building
emerge -pt noctalia --aur --deep   # same, but nested all the way down
emerge deepin-session --ask --tree --deep=3   # install, tree capped at 3 levels
```

```
[ebuild  N ] noctalia-5.1.0-1.1
[ebuild  N ] `-- libqalculate-5.12.0-1.1
[ebuild  N ]   `-- sdbus-cpp-2.3.1-1.1
```

A dependency pacman pulled in that isn't literally named in anyone's "Depends On" (resolved via a `provides`, or beyond `--deep`'s reach) still shows, just without a specific parent to nest under - real information, not a fabricated tree. For a pure `--aur` install, the plan step doesn't resolve transitive AUR-only dependencies yet, so `-t` there is currently flat, the same as without it.

## Merge/unmerge log

`/var/log/emerge.log` - Portage's `emerge.log`, for pacman/AUR/ABS atoms. Append-only, one line per event, written the same way other root-owned `/etc/portage/*` state is (`sudo tee`, best-effort - a failed log write never fails the merge/unmerge that already happened).

```
2026-09-21 08:51:44  MERGE    aur    noctalia-5.1.0-1.1        (12s)
2026-09-21 08:52:10  MERGE    aur    libqalculate-5.12.0-1.1   (9s)
2026-09-21 09:03:02  MERGE    repo   nano-8.0-1  vim-9.1-1     (4s)
2026-09-21 09:10:05  UNMERGE  -      old-package-2.0-1
```

Duration is only ever real: AUR/ABS packages build one at a time in this tool's own loop, so each gets its own timer; a `pacman -S`/`-Syu` transaction is one exit code for the whole batch, so that line carries every atom in it and the batch's own total, not a guessed-at split. `--info` shows a running total (merge/unmerge counts, cumulative build time).

## Usage

```bash
emerge neovim                              # official repos, falls back to AUR
emerge neovim-git --aur                    # AUR only (AUR-only deps built recursively)
emerge ayugram-desktop-bin --aur --sudoloop   # same, but keep sudo warm through the whole recursive build
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
emerge @world                              # provision: install missing entries from world
emerge @world --err-install                   # also retry Err/ (unknown-source) entries normally
emerge -u @world                           # full system upgrade (repos + AUR); does not read world
emerge -u @world --refresh                 # upgrade with forced -Syy first
emerge --sync                              # sync package databases
emerge @game-kit                           # custom set from /etc/portage/sets/game-kit.set
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
emerge -1 htop                             # install as dependency (not added to world)
emerge -n neovim                           # do not reinstall if present (--noreplace)
emerge -p neovim                           # dry run
emerge -a neovim                           # ask before acting
emerge --select neovim                     # add to world without installing
emerge --deselect neovim                   # remove from world without unmerging
emerge --prune                             # remove packages not in world
emerge --regen                             # regenerate package metadata cache
emerge --regen-world                       # re-resolve repo prefixes in world
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
emerge -u --exclude linux                   # upgrade, hold one package back
emerge -u --keep-going                      # upgrade, don't stop on the first failure
emerge --revdep-rebuild                     # find + fix broken library links
emerge --revdep-rebuild -p                  # same, list only
```

## Sandbox

**AUR and ABS** builds run the PKGBUILD functions `pkgver()` / `prepare()` / `build()` / `check()` / `package()` inside a [`bwrap`](https://github.com/containers/bubblewrap) namespace (when `bubblewrap` is installed):

- fake `$HOME` (no SSH keys, GPG secret keyring, browser profile);
- writes only inside the build directory (plus configured `PKGDEST` / `SRCDEST` / `SRCPKGDEST` / `BUILDDIR`);
- network shared by default (`--share-net`);
- other namespaces (user / IPC / PID / UTS / cgroup) torn down;
- `package()`'s `fakeroot` runs in its own nested, uid-0-mapped `bwrap` (fixes `fchownat` failing under a plain user namespace); the shim script for it lives under `/var/tmp`, outside the writable build dir, so `prepare()`/`build()` can't tamper with it before `package()` runs it.

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

`--batchinstall <FILE>` reads a plain-text package list and folds it into the normal package list before anything runs - same format as a [custom set](#custom-sets) (one atom per line, `#` comments allowed), but from any path you give directly, no need to register it under `/etc/portage/sets/` first.

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

`--install-pkgbuild <PATH>` builds and installs an arbitrary local PKGBUILD checkout through the same pipeline as an AUR/ABS install - scanner, then the bwrap sandbox - instead of a bare, unaudited `makepkg -si`. `PATH` must contain a `PKGBUILD`; if there's no `.SRCINFO` yet, one is generated (`makepkg --printsrcinfo`, same as `--edit` does) so dependencies resolve statically. Combines with `--pkgbuild-view`, `-a`/`--ask`, `-1`/`--oneshot`, `--skippgp`, `--no-sandbox`, `--unshare-net-build`, `--skip-srcinfo-regen`; not compatible with any package/`@set` argument or `--aur`/`--abs`. Recorded in `world` with the `Err/` prefix (source unknown - see [world](#worldset)) unless `-1`/`--oneshot` is also given.

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

Arch has no preserved-libs tracking. `@preserved-rebuild` collects every installed package’s declared dependencies (`pacman -Qi`), feeds the deduplicated list to `pacman -T`, and offers to reinstall unsatisfied ones via `pacman -S --asdeps` (so they don’t look “explicit” and get vacuumed into world by a later `--regen-world-from-explicit`).

## --revdep-rebuild

Not the same check as `@preserved-rebuild` above, which only sees *declared* dependencies via `pacman -T`. `--revdep-rebuild` reads installed ELF files directly (`DT_NEEDED`, `DT_RPATH`/`DT_RUNPATH`) and finds binaries linking against a `.so` that no longer exists - the classic post-`openssl`/`icu`/`boost`-bump breakage Gentoo handles via preserved-libs and Arch doesn't, out of the box.

Foreign (AUR/local) packages get rebuilt through the normal sandboxed path; official-repo packages can't be rebuilt on a binary distro, so their missing libraries are looked up in the file database (`pacman -F`) and the packages that provide them are offered for install instead. `--exclude` and the [mask](#mask-etcemergemask) both apply.

```bash
emerge --revdep-rebuild       # scan, then ask before fixing anything
emerge --revdep-rebuild -p    # scan only, list what's broken
```

## world

Path: `/etc/portage/world`. Every explicitly installed package is added automatically and removed on unmerge. Editable by hand (one package per line).

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

- **`emerge @world` (no `-u`)** - *provision*. Installs whatever `world` lists that isn’t already on the system; already-installed packages are left alone. Prefix decides the source. Keep `world` in dotfiles, clone on a fresh machine, run `emerge @world`.
- **`emerge -u @world`** (or `emerge -u`) - *upgrade*. Upgrades everything already installed (repos + AUR). Does not read `world`.
- **`emerge --sync`** - database refresh only (`-Sy` / `-Syy`).

## Custom sets

`/etc/portage/sets/<name>.set` is invoked as `@<name>`:

```
# /etc/portage/sets/game-kit.set
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

1. **AST** (`tree-sitter-bash`) - `curl|sh`, `base64`/`xxd | sh`, `source <(curl …)`, `eval "$(curl …)"`, `sudo`/`pkexec`/`doas`, python inline-exec, `sh`/`bash`/`dash`/`ash` `-c` with suspicious payload, top-level command substitution (`` `...` `` / `$(...)` outside function bodies - e.g. leftover markdown backticks in `pkgdesc`), etc. Trivial text evasions (string concatenation, absolute paths, `env` wrappers) do not slip through.
2. **Line heuristics** - `chmod 777`, raw IPv4 literals, foreign package managers (`npm install …`), paste sites, `.onion` addresses, anti-sandbox/anti-debugger probes (`TracerPid`, `LD_PRELOAD`/`LD_LIBRARY_PATH`), known IOCs (Atomic Arch, openconnect-sso, 2018 `compromised.txt`, …).

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
| `-u` | Full system upgrade (does not read world) |
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
| `--select` | Add to world without installing |
| `--deselect` | Remove from world without unmerging |
| `--prune` | Remove packages not in world |
| `--regen` | Regenerate metadata cache |
| `--regen-world` | Re-resolve repo prefixes in world |
| `--regen-sets <SET>` | Re-resolve prefixes for one custom set |
| `--regen-sort` | With `--regen-sets` - also sort the file |
| `--regen-world-from-explicit` | Migration: seed world from `pacman -Qeq` |
| `--err-install` | On `@world`, also retry `Err/` entries |
| `--resume` | Resume interrupted transaction |
| `--skipfirst` | Skip first package on resume |
| `--undo` | Undo last install/unmerge |
| `--clean-source-cache` | Wipe persistent VCS source cache |
| `--batchinstall <FILE>` | Mass-install from a package list file |
| `--list-sets` | List available `@sets` |
| `--exclude <ATOM>` | Leave a package out of this run (repeatable/comma-separated) |
| `--keep-going` | Don't stop a batch at the first failure; summarize at the end |
| `--revdep-rebuild` | Find + fix binaries linking against a missing library |
| `-t`, `--tree` | Show the plan as a dependency tree |
| `-D`, `--deep[=N]` | With `-t`: nest N levels deep, or every level if bare |
| `--ignore-default-opts` | Ignore `EMERGE_DEFAULT_OPTS` from make.conf for this run |
| `-h` / `--help` | Help |
| `-V` / `--version` | Version |

The `emerge(1)` man page is generated from the CLI via `clap_mangen` (`emerge --gen-manpage`), so it cannot drift from `--help`.
</details>

## Author/License
Copyright (C) 2026 **Undercat037**
GPL-3.0 license