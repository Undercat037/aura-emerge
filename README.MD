<div align="center">
  <img src="misc/logo/slash-logo-128x128.svg" width="128" alt="aura-emerge logo">

# Aura-Emerge Secure Package Manager

[![AE_Site](https://img.shields.io/badge/AE_Site-333333?style=flat-square&logo=bookstack&logoColor=FFFFFF)](https://undercat037.github.io/aura-emerge/)
[![AUR](https://img.shields.io/badge/AUR-333333?style=flat-square&logo=arch-linux&logoColor=1793D1)](https://aur.archlinux.org/packages/aura-emerge)
[![GitHub](https://img.shields.io/badge/GitHub-333333?style=flat-square&logo=github&logoColor=FFFFFF)](https://github.com/Undercat037/aura-emerge)
[![GitLab](https://img.shields.io/badge/GitLab-333333?style=flat-square&logo=gitlab&logoColor=FC6D26)](https://gitlab.com/Undercat037/aura-emerge)

en | [ua](UA-README.MD) | [ru](RU-README.MD)
</div>

> A standalone Gentoo-style `emerge` package manager for Arch Linux. Installs from official repos, the AUR and ABS; scans PKGBUILDs for supply-chain attack patterns before building; runs untrusted build steps inside a `bwrap` sandbox.

Packages are tracked in a `world` file - install once, track forever.

## Installation
**AUR helper**
```bash
paru -S aura-emerge 
```

or with plain **makepkg**:

```bash
git clone https://aur.archlinux.org/aura-emerge.git
cd aura-emerge
makepkg -si
```

<details>
<summary><h3>Build from source (unstable)</h3></summary>

**Git makepkg**

```bash
git clone https://github.com/Undercat037/aura-emerge.git
cd aura-emerge
make install           
```
**Manual**

```bash
cargo build --release
sudo cp target/release/aura-emerge /usr/bin/emerge
sudo ln -sf /usr/bin/emerge /usr/bin/portageq
emerge --gen-completions bash | sudo tee /usr/share/bash-completion/completions/emerge
emerge --gen-completions zsh  | sudo tee /usr/share/zsh/site-functions/_emerge
emerge --gen-completions fish | sudo tee /usr/share/fish/vendor_completions.d/emerge.fish
emerge --gen-manpage | sudo tee /usr/share/man/man1/emerge.1 >/dev/null
```
</details>

**Dependencies:** `git`, `bubblewrap` (build sandbox for AUR/ABS)     
**Optional:** `devtools` (`--abs` support), `gnupg` (`--autopgp`, automatic PGP key import)

## Usage

```bash
# Install
emerge neovim                       # official repos, falls back to AUR
emerge neovim-git --aur             # AUR only (AUR-only deps are built recursively)
emerge neovim --repos               # official repos only, never touch the AUR
emerge neovim --abs                 # build from ABS (pkgctl) in the sandbox
emerge -p neovim                    # dry run
emerge -a neovim                    # ask before acting
emerge -1 htop                      # install as a dependency (not added to world)

# Remove
emerge -C neovim                    # unmerge
emerge -c                           # remove orphans (depclean)
emerge --prune                      # remove packages not in world

# Update
emerge --sync                       # refresh package databases
emerge -u                           # full upgrade (repos + AUR)
emerge -u --devel                   # ...and rebuild -git/-hg/-svn/-bzr packages whose upstream moved
emerge -u --exclude linux --keep-going

# World and sets
emerge @world                       # install whatever is in world but missing
emerge @game-kit neovim             # custom set + plain package
emerge --select neovim              # add to world without installing
emerge --deselect neovim            # remove from world without unmerging

# Search and info
emerge -s neovim                    # search: repos, then ABS, then AUR
emerge -S neovim                    # search by description
emerge -sv neovim                   # detailed package info
emerge --info                       # system info
emerge --news                       # Arch Linux news

# Security
emerge neovim-git --aur --pkgbuild-view      # view/diff the PKGBUILD before building
emerge neovim-git --aur --edit               # edit the PKGBUILD first
emerge --scan neovim-git --aur               # audit only, no build
emerge --install-pkgbuild ~/src/my-pkg       # build a local PKGBUILD through scanner + sandbox

# Recovery
emerge --resume                     # resume an interrupted transaction
emerge --undo                       # undo the last install/unmerge
emerge --revdep-rebuild             # fix binaries linked to missing libraries
```

The full flag table is at the bottom of this page.

## Configuration

### Gentoo → Arch: `make.conf` equivalents

Arch has no USE flags, but compilation can be tuned much like [Gentoo](https://wiki.gentoo.org/wiki/Main_Page)'s `/etc/portage/make.conf` - through `/etc/makepkg.conf`:

| Gentoo (`make.conf`)         | Arch (`makepkg.conf`)         | Purpose                     |
|------------------------------|-------------------------------|-----------------------------|
| `CFLAGS="-O2 -march=native"` | `CFLAGS="-O2 -march=native"`  | C compiler flags            |
| `CXXFLAGS="-O2 -march=..."`  | `CXXFLAGS="-O2 -march=..."`   | C++ compiler flags          |
| `RUSTFLAGS="-C opt-level=3"` | `RUSTFLAGS="-C opt-level=3"`  | Rust compiler flags         |
| `MAKEOPTS="-j$(nproc)"`      | `MAKEFLAGS="-j$(nproc)"`      | Parallel build jobs         |
| `USE="lto"`                  | `OPTIONS+=(lto)`              | Link-time optimisation      |
| `FEATURES="ccache"`          | `BUILDENV+=(ccache)`          | Build cache                 |
| `PORTDIR` / `PKGDIR`         | `PKGDEST`, `SRCDEST`          | Build artefact directories  |

See [GCC optimization flags](https://gcc.gnu.org/onlinedocs/gcc/Optimize-Options.html), [rustc codegen options](https://doc.rust-lang.org/rustc/codegen-options/index.html) and the [**example makepkg.conf**](misc/makepkg.conf).

### `/etc/portage/make.conf`

aura-emerge's own config. It overrides `/etc/makepkg.conf` for builds made by aura-emerge, without touching the original file.

```bash
# Default flags, added before whatever you type (the command line wins on conflict).
# Skip them for one run with --ignore-default-opts.
EMERGE_DEFAULT_OPTS="--pkgbuild-view --unshare-net-build"

# Build flags (applied through a generated makepkg.conf)
CFLAGS="-march=native -O2 -pipe"
CXXFLAGS="$CFLAGS"
RUSTFLAGS="-C target-cpu=native"
MAKEFLAGS="-j$(nproc)"
NINJAFLAGS="-j$(nproc)"
OPTIONS=(strip docs !debug lto)
BUILDENV=(!distcc color check !sign)

# Portage-style switches, mapped to makepkg
FEATURES="ccache -test"
```

**Keys:** `EMERGE_DEFAULT_OPTS`, `FEATURES`, `CFLAGS`, `CXXFLAGS`, `CPPFLAGS`, `LDFLAGS`, `RUSTFLAGS`, `MAKEFLAGS`, `NINJAFLAGS`, `BUILDENV`, `OPTIONS`.
**Syntax:** bash-style `KEY="..."` or `KEY=(a b c)`. Strings and lists are interchangeable, and values are shell-expanded (`$(nproc)` and `$CFLAGS` work; use single quotes to keep a literal `$`).

`FEATURES` tokens (`-name` turns one off) are mapped to their makepkg twins. Anything you set yourself in `BUILDENV`/`OPTIONS` wins.

| `FEATURES`   | makepkg             |
|--------------|---------------------|
| `ccache`     | `BUILDENV` `ccache` |
| `distcc`     | `BUILDENV` `distcc` |
| `test`       | `BUILDENV` `check`  |
| `nostrip`    | `OPTIONS` `!strip`  |
| `splitdebug` | `OPTIONS` `debug`   |

`EMERGE_DEFAULT_OPTS` accepts modifier flags only (`--ask`, `--verbose`, `--devel`, `--tree`, `--searchdesc`, `--exclude=<atom>`, ...), so it can't turn every run into `--unmerge`. Conflicting pairs are rejected if both are set: `--aur`/`--abs`, `--aur`/`--repos`, `--no-sandbox`/`--unshare-net-build`, `--skippgp`/`--autopgp`, `--select`/`--deselect`. If a config default conflicts with the command line, the config default is dropped. `--abs` and `--repos` do **not** conflict.

`emerge --info` shows what was read and the final value of each override. [**→ Example make.conf**](misc/make.conf)

### `package.mask`

Packages this machine must never install - checked even for transitive AUR dependencies and even when requested by name. A masked package hard-stops the request: no prompt, no override flag.

`/etc/portage/package.mask` is a file or a directory (every regular file inside is read in name order; dotfiles are skipped; symlinks are refused). One entry per line, `*` globs and an optional repo prefix allowed, `# comment` at the end.

```
ayugram-desktop-bin            # a specific package, any source
aur/*-bin                      # never install AUR -bin packages
*-git                          # no devel packages on this box
extra/nano                     # a specific package from a specific repo
```

Not the same as `--exclude`: a mask is permanent, `--exclude` is for one run.

### `package.env`

Per-package build flags. Each entry is an atom followed by one or more env file names from `/etc/portage/env/`. Like `package.mask`, `/etc/portage/package.env` can be a file or a directory.

```
# /etc/portage/package.env/neovim
abs/neovim  lto.conf            # repo prefix and '*' work as in package.mask
aur/*-git   no-lto  fast        # several envs, applied in the order named
```

```bash
# /etc/portage/env/lto.conf  (make.conf syntax; only build keys count)
CFLAGS="-march=znver3 -O2 -pipe"
MAKEFLAGS="-j16"
OPTIONS="strip docs !debug lto"
```

- **Layering:** `make.conf` → matching `package.env` entries → each env file in the order named. The last layer to set a key wins, and the key is replaced whole, not merged (an `OPTIONS` in an env file replaces the one from `make.conf`). The build prints which layer won.
- **Matching:** by pkgbase or any pkgname.
- **Scope:** AUR/ABS builds only. `EMERGE_DEFAULT_OPTS` in an env file is ignored with a warning.
- **Errors:** a symlink, an invalid entry or a missing env file is skipped with a warning naming the file and line; the rest still applies.

## Features

### `--exclude` and `--keep-going`

`--exclude <atom>` (repeatable or comma-separated) leaves a package out of the current run: installs, `-u`, `@world`, `--depclean`/`--prune` (protects it from removal) and `--revdep-rebuild`.

`--keep-going` doesn't stop a batch (`@world`, `-u`, `--batchinstall`, multi-package installs) at the first failure. It collects the failures, prints a summary and exits non-zero. A later `--resume` retries only what failed.

```bash
emerge -u --exclude linux --exclude aur/ayugram-desktop-bin   # upgrade everything except these
emerge -u --keep-going                                        # don't stop on one bad AUR package
emerge --resume                                               # retry just what failed
```

### `-t` / `--tree` and `--deep`

Shows the plan (with `-p`, or right before a real install) as a dependency tree. By default one level is nested; `--deep` goes all the way down, `--deep=N` stops at N levels.

```bash
emerge -apt noctalia --aur           # ask + pretend + tree
emerge -pt noctalia --aur --deep     # full depth
```

```
[ebuild  N ] noctalia-5.1.0-1.1
[ebuild  N ] `-- libqalculate-5.12.0-1.1
[ebuild  N ]   `-- sdbus-cpp-2.3.1-1.1
```

A dependency that isn't named in anyone's "Depends On" (resolved via `provides`, or beyond `--deep`) is still shown, just without a parent. For a pure `--aur` install, transitive AUR-only dependencies aren't resolved at the plan step yet, so the tree is flat there.

### Merge log

`/var/log/emerge.log` - append-only, one line per event. A failed log write never fails the merge itself.

```
2026-09-21 08:51:44  MERGE    aur    noctalia-5.1.0-1.1        (12s)
2026-09-21 09:03:02  MERGE    repo   nano-8.0-1  vim-9.1-1     (4s)
2026-09-21 09:10:05  UNMERGE  -      old-package-2.0-1
```

AUR/ABS packages are built one by one, so each has its own timer. A `pacman -S`/`-Syu` run is a single transaction, so its line lists all atoms with the total time. `--info` shows totals (merge/unmerge counts, cumulative build time).

### Batch install

`--batchinstall <FILE>` installs from a plain-text list (one atom per line, `#` comments allowed). Same format as a [custom set](#custom-sets), but from any path - no registration needed. Combines with any other flag (`-p`, `-a`, `--aur`, `--abs`, `--repos`, ...).

```bash
emerge --batchinstall pkgs.txt
```

### PKGBUILD review and scan

**`--pkgbuild-view`** shows the PKGBUILD right before the build and asks for confirmation: a diff against the last copy you saw (cached in `~/.cache/aura-emerge/pkgbuild-view/`), or the whole file the first time. If you decline, it offers to open `$EDITOR` instead of just skipping. Works after `--edit`, so you see your own changes. Applies only to directly requested AUR/ABS packages - not to recursive AUR dependencies or batch paths (`@world`, `-u`, `--undo`).

**`--install-pkgbuild <PATH>`** builds and installs a local PKGBUILD checkout through the same pipeline as AUR/ABS (scanner → sandbox) instead of a bare `makepkg -si`. `PATH` must contain a `PKGBUILD`; a missing `.SRCINFO` is generated. It combines with `--pkgbuild-view`, `-a`, `-1`, `--skippgp`, `--no-sandbox`, `--unshare-net-build` and `--skip-srcinfo-regen`, but not with package/`@set` arguments or `--aur`/`--abs`. The package is recorded in `world` as `Err/` (source unknown) unless `-1` is given.

**`--scan`** audits a PKGBUILD (+ `.install`) without building or installing: it fetches from cgit for an AUR package, or reads a local checkout with `--install-pkgbuild <PATH> --scan`. Same report as the pre-build scan, minus the "Continue anyway?" prompt. Exits non-zero on any finding, so it works in scripts. Not available for `--abs` yet.

```bash
emerge neovim-git --aur --pkgbuild-view
emerge --install-pkgbuild ~/src/my-pkg --pkgbuild-view --skippgp
emerge --scan neovim-git --aur
```

### Devel packages (`-git`, `-hg`, `-svn`, `-bzr`)

The version shown on an AUR page only changes when the maintainer re-pushes the package, so plain `-u` often misses upstream changes in devel packages.

`-u --devel` (upgrade) and `--check-devel` (report only) fix that: for each installed devel package they read the `git+` source URL from its PKGBUILD, run `git ls-remote` (no clone needed) and compare with the hash seen last time (`~/.cache/aura-emerge/devel.state`). Git only for now; `-hg`/`-svn`/`-bzr` are reported as "couldn't determine", not silently skipped.

```bash
emerge -u --devel        # upgrade + rebuild devel packages whose upstream moved
emerge --check-devel     # just report
```

### Source cache

For `-git`/`-hg`/`-svn` packages, `SRCDEST` defaults to `~/.cache/aura-emerge/sources` (or `$XDG_CACHE_HOME/aura-emerge/sources`), so makepkg does an incremental fetch instead of a full re-clone. If `makepkg.conf` already sets `SRCDEST`, that wins. Wipe the cache with `emerge --clean-source-cache`.

### `@preserved-rebuild` and `--revdep-rebuild`

Arch has no preserved-libs tracking. Two tools cover the gap:

- **`@preserved-rebuild`** collects the *declared* dependencies of every installed package (`pacman -Qi`), checks them with `pacman -T` and offers to reinstall the missing ones with `--asdeps` (so they don't get pulled into world by a later `--regen-world-from-explicit`).
- **`--revdep-rebuild`** reads the installed ELF files (`DT_NEEDED`, `DT_RPATH`/`DT_RUNPATH`) and finds binaries linked to a `.so` that no longer exists - the classic breakage after an `openssl`/`icu`/`boost` bump. AUR/local packages are rebuilt through the sandbox; for official packages (which can't be rebuilt here) the missing library is looked up with `pacman -F` and the providing package is offered for install. `--exclude` and the [mask](#packagemask) apply.

```bash
emerge @preserved-rebuild -p     # list only
emerge --revdep-rebuild          # scan, then ask before fixing
emerge --revdep-rebuild -p       # scan only
```

### `world`

`/etc/portage/world` lists every explicitly installed package. Entries are added on install, removed on unmerge, and can be edited by hand (one per line).

```
core/nano
extra/firefox
aur/ayugram-desktop-bin
abs/nano
chaotic-aur/visual-studio-code-bin
```

**Prefixes:** official repo name, `aur/`, `abs/`, `Err/` (source unknown).

> **About `Err/`:** used for a local build with no traceable origin, and for an `abs/`/`aur/` entry that stopped resolving. `--regen-world` and `--regen-sets` keep a known `abs/`/`aur/` prefix instead of falling back to `Err/`.

If one name in a batch is missing, the rest still install; the missing ones are listed at the end with a non-zero exit code.

**`@world` has two meanings:**

- `emerge @world` - *provision*: installs what `world` lists but isn't installed yet. Keep `world` in your dotfiles, clone on a fresh machine, run `emerge @world`.
- `emerge -u` (or `emerge -u @world`) - *upgrade*: upgrades everything installed (repos + AUR). Doesn't read `world`.
- `emerge --sync` - refreshes databases only (`-Sy` / `-Syy`).

### Custom sets

`/etc/portage/sets/<name>` (or `<name>.set`) is invoked as `@<name>`. One package per line, `#` comments allowed.

```
# /etc/portage/sets/game-kit
steam
lutris
gamemode
aur/protonup-qt
```

```bash
emerge @game-kit @dev-tools neovim    # sets combine freely
emerge --list-sets                    # list sets (also: emerge @<TAB>)
emerge -s @game-kit                   # where each package comes from: repo / aur / not found
```

If both `game-kit` and `game-kit.set` exist, `@game-kit` is rejected as ambiguous. `@world` and `@preserved-rebuild` are actions, so `-s @world` and `-s @preserved-rebuild` give an error.

### `--undo`

Reverses the last successful install or unmerge (one step of history):

- **after install** - removes only packages that were new in that run;
- **after unmerge** - reinstalls from the recorded source (`Err/`/`abs/` are only listed);
- **after `-u`** - not supported (no version snapshot).

### News

`emerge --news` reads the official [Arch Linux news feed](https://archlinux.org/feeds/news/). Read state is kept in `~/.cache/aura-emerge/news.state` (no root needed).

```bash
emerge --news        # list, unread marked [N]
emerge --news 3      # read item 3
emerge --news all    # mark all as read
```

## Security

### PKGBUILD scanning

Before every AUR install, the raw `PKGBUILD` (and any `.install` hook) is fetched from cgit and scanned in two layers:

1. **AST** (`tree-sitter-bash`): `curl|sh`, `base64`/`xxd | sh`, `source <(curl …)`, `eval "$(curl …)"`, `sudo`/`pkexec`/`doas`, inline exec for python/perl/ruby/node/lua, `sh -c` with a suspicious payload, top-level command substitution, string-concatenation tricks, etc.
2. **Line heuristics:** `chmod 777`, raw IPv4, foreign package managers (`npm install …`), paste sites, `.onion`, anti-sandbox probes, known campaign IOCs (Atomic Arch, openconnect-sso, 2018 `compromised.txt`, …).

Findings are `Suspicious` or `ConfirmedIoc`. A hit doesn't hard-block: you get the file/line, a cgit link and `Continue anyway? [y/N]`.

### Sandbox

**AUR and ABS** builds run `pkgver()` / `prepare()` / `build()` / `check()` / `package()` inside [`bwrap`](https://github.com/containers/bubblewrap). It's required; the only way out is an explicit `--no-sandbox`.

- empty tmpfs over `/home` and `/run` (no real `$HOME`, no session D-Bus / ssh-agent / gpg-agent);
- `--clearenv` plus a small allowlist (`PATH`, `TERM`, `LANG`, build destinations, …);
- a copy of the GnuPG keyring with public keys only;
- PKGBUILD and `*.install` re-mounted read-only;
- writes only to the build dir (+ configured `PKGDEST` / `SRCDEST` / …);
- network shared by default; other namespaces torn down;
- `fakeroot` in `package()` runs in a nested uid-0 `bwrap`.

| Flag | Effect |
|------|--------|
| `--no-sandbox` | Disable bwrap; plain `makepkg -si` |
| `--unshare-net-build` | Network only for download/`prepare()`; `build()`/`check()`/`package()` run offline |

If `bubblewrap` is missing, the build is refused.

**Third gate before install:** the finished `*.pkg.tar.*` is audited with `bsdtar` for setuid/setgid, `.INSTALL` scripts and sensitive paths (alpm hooks, systemd units, sudoers, udev, …). `.INSTALL` is re-scanned with the PKGBUILD scanner. With `-a` you must confirm; with `--noconfirm` it warns and continues.

Dependency install (`pacman -S`) and the final `pacman -U` run **outside** the sandbox - they don't execute PKGBUILD code.

### Other hardening

- Arch news text is sanitized (no ANSI/bidi injection into the terminal or `news.state`).
- Config, mask, env and world files are opened with `O_NOFOLLOW`, so a symlink can't be raced in.
- Rust dependencies are checked with [`cargo deny`](https://github.com/EmbarkStudios/cargo-deny) (`make deny`).

<details>
<summary><h3>Testing the scanner (after install)</h3></summary>

```bash
git clone https://github.com/Undercat037/aura-emerge.git
cd aura-emerge
emerge --install-pkgbuild ./scaner-test --ask
```
</details>

<details>
<summary><h2>Flags</h2></summary>

| Flag | Description |
|------|------|
| `-s`, `--search` | Search packages |
| `-S`, `--searchdesc` | Search by description |
| `--search-all` | With `-s`: list the same name from every repo (like `pacman -Ss`) |
| `-sv` | Detailed package info |
| `-v`, `--verbose` | Verbose output / detailed search info (`-sv`) |
| `-u`, `--update` | Full system upgrade (doesn't read world) |
| `--devel` | With `-u`: also rebuild `-git`/`-hg`/`-svn`/`-bzr` packages whose upstream moved |
| `--check-devel` | Report devel packages with upstream changes (no rebuild) |
| `-C`, `--unmerge` | Remove packages (unmerge) |
| `-c`, `--depclean` | Remove orphans (depclean) |
| `--show-protected` | With `-c`: list orphans kept only because they're in world |
| `-p`, `--pretend` | Pretend (dry run) |
| `-a`, `--ask` | Ask before acting |
| `--sudoloop` | Keep the sudo timestamp warm (refreshed every 60 s) for the whole run |
| `-1`, `--oneshot` | Install as dependency (skip world) |
| `-n`, `--noreplace` | Don't reinstall if present (`pacman --needed`) |
| `--with-optdeps` | Also install optional dependencies of official-repo packages |
| `-N`, `--newuse` | Include installed packages with changed USE flags |
| `-e`, `--emptytree` | Reinstall all world packages |
| `--aur` | Force AUR only (AUR-only dependencies are always built recursively) |
| `--repos` | Official repos only (never AUR) |
| `--abs` | Build from ABS (`pkgctl`) in the bwrap sandbox |
| `--no-sandbox` | Disable bwrap for AUR/ABS; plain unisolated makepkg |
| `--unshare-net-build` | No network during build()/check()/package() (`prepare()` still has network) |
| `--skippgp` | Skip PGP checks on ABS builds (`--skippgpcheck`) |
| `--autopgp` | Auto-import missing PGP keys before `--abs` builds |
| `--edit` | Edit PKGBUILD in `$EDITOR` before building (top-level package only); regenerates `.SRCINFO` after save |
| `--skip-srcinfo-regen` | With `--edit`: don't regenerate `.SRCINFO` |
| `--pkgbuild-view` | Show/diff PKGBUILD and confirm before building (top-level package only; offers `$EDITOR` on decline) |
| `--install-pkgbuild <PATH>` | Build and install a local PKGBUILD checkout (scanner + bwrap); recorded in world as `Err/` unless `-1` |
| `--scan` | Audit PKGBUILD/.install only, no build/install; non-zero exit on findings (AUR or `--install-pkgbuild`) |
| `--sync` | Sync package databases |
| `--refresh` | Force `-Syy` (with `--sync` or `-u`) |
| `--info` | System info, like Gentoo's `emerge --info` |
| `--news [N\|all]` | Arch Linux news: bare lists items, `N` reads one, `all` marks all as read |
| `--check-news [N\|all]` | Alias for `--news` |
| `--select` | Add to world without installing |
| `--deselect` | Remove from world without unmerging |
| `--check-world` | List world entries that are missing or not explicitly installed |
| `--prune` | Remove packages not in world |
| `--regen` | Regenerate package metadata cache |
| `--regen-world` | Re-resolve repo prefixes in world |
| `--regen-sets <SET>` | Re-resolve prefixes in a custom set (`@game-kit` or `game-kit`) |
| `--regen-sort` | With `--regen-sets`: also sort the file (drops comments and grouping) |
| `--regen-world-from-explicit` | Migration: seed world from `pacman -Qeq` (one-time) |
| `--err-install` | With `@world`: also install `Err/` (unknown-source) entries |
| `--resume` | Resume an interrupted transaction |
| `--skipfirst` | With `--resume`: skip the first package |
| `--undo` | Undo the last install/unmerge (one step; `-u` is not reversible) |
| `--clean-source-cache` | Wipe the persistent VCS source cache and exit |
| `--batchinstall <FILE>` | Mass-install from a text list (same format as a custom set) |
| `--list-sets` | List available `@sets` (also used by shell completion) |
| `--exclude <ATOM>` | Leave a package out of this run (repeatable/comma-separated) |
| `--keep-going` | Don't stop a batch at the first failure; report failures at the end |
| `--revdep-rebuild` | Rebuild packages whose binaries link against a missing library |
| `-t`, `--tree` | Show the plan as a dependency tree |
| `-D`, `--deep[=N]` | With `-t`: nest N levels, or all levels if bare |
| `--jobsr <N>` | Max parallel official-repo installs (default 1) |
| `--jobsa <N>` | Max parallel AUR/ABS builds (default 1) |
| `--jobs <N>` | Alias for `--jobsr` (Portage-style) |
| `--verbose-conflicts` | Verbose slot/conflict info (informational) |
| `--ignore-default-opts` | Ignore `EMERGE_DEFAULT_OPTS` from make.conf for this run |
| `--debug` | Show makepkg/compiler output live (also `AE_DEBUG=1`); by default it's hidden to keep the `>>>` lines readable, and a failed build still dumps the captured log |
| `--log <PATH>` | Write a session log (command, emerge.log lines, full build output) for bug reports - no `| tee` needed |
| `-h`, `--help` | Help |
| `-V`, `--version` | Version |

The `emerge(1)` man page is generated from the CLI (`emerge --gen-manpage`), so it can't drift from `--help`.
</details>

## Author / License
Copyright (C) 2026 **Undercat037**
GPL-3.0 license
