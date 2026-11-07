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

**Sections:** [Installation](#installation) · [Usage examples](#usage-examples) · [Configuration](#configuration) · [Working with packages](#working-with-packages) · [Repositories](#repositories) · [Security](#security) · Flags (at the end)

## Installation

```bash
paru -S aura-emerge          # via an AUR helper
```

or with plain makepkg:

```bash
git clone https://aur.archlinux.org/aura-emerge.git
cd aura-emerge
makepkg -si
```

<details>
<summary><h3>Building from source (unstable, beta versions)</h3></summary>

```bash
git clone https://github.com/Undercat037/aura-emerge.git
cd aura-emerge
make install            # or by hand:
```

```bash
cargo build --release
sudo install -Dm755 -o root -g root target/release/aura-emerge /usr/bin/emerge
sudo ln -sf /usr/bin/emerge /usr/bin/portageq
sudo ln -sf /usr/bin/emerge /usr/bin/eselect
emerge --gen-completions bash | sudo tee /usr/share/bash-completion/completions/emerge
emerge --gen-completions zsh  | sudo tee /usr/share/zsh/site-functions/_emerge
emerge --gen-completions fish | sudo tee /usr/share/fish/vendor_completions.d/emerge.fish
emerge --gen-manpage | sudo tee /usr/share/man/man1/emerge.1 >/dev/null
sudo install -Dm644 misc/eselect.bash-completion /usr/share/bash-completion/completions/eselect
sudo install -Dm644 misc/_eselect /usr/share/zsh/site-functions/_eselect
sudo install -Dm644 misc/eselect.fish /usr/share/fish/vendor_completions.d/eselect.fish
```
</details>

**Dependencies:** `git`, `sudo`, `bubblewrap` (AUR/ABS build sandbox), `pacman` (libalpm, makepkg), `gnupg` (key import for the built-in `pacman-key` implementation and for `--autopgp` with ABS).

**Optional:** `devtools` (`--abs` support).

## Usage examples

```bash
# Installing
emerge neovim                       # official repos, falling back to the AUR
emerge neovim --nodeps              # install the package without its dependencies
emerge neovim-git --aur             # AUR only (AUR-only dependencies are built recursively)
emerge neovim --repos               # official repos only
emerge neovim --abs                 # build from ABS (pkgctl) in the sandbox
emerge aur/neovim-git abs/nano cachyos-extra-v3/neovim   # per-package source via a prefix
emerge -pt neovim                   # dry run, plan as a tree
emerge -a neovim                    # ask before acting
emerge -1 htop                      # as a dependency (does not go into world)
emerge -at aur/setrixtui abs/nano @fonts --jobs=7 --jobsa=2 # several packages at once: 7 at a time from repos, 2 AUR/ABS builds at a time

# Removing
emerge -C neovim                    # remove a package (--unmerge)
emerge -c                           # remove orphans (--depclean)
emerge -cp --show-protected         # orphans, plus those kept back by world, package.protect or the selected profile
emerge --prune                      # remove everything that is not in world

# Updating
emerge --sync                       # refresh package databases
emerge -u                           # full upgrade (repos + AUR)
emerge -u --devel                   # ...and rebuild -git/-hg/-svn/-bzr packages with a newer upstream
emerge -u --exclude linux --keep-going # exclude the `linux` package from the upgrade

# World and sets
emerge @world                       # install everything from world that is missing
emerge @desktop neovim              # a custom set + a regular package
emerge --select neovim              # add to world without installing
emerge --deselect neovim            # remove from world without uninstalling

# Search and info
emerge -s neovim                    # search: repos => ABS => AUR
emerge -S neovim                    # search by description
emerge -sv neovim                   # verbose
emerge -s abs/nano                  # ABS only (aur/ and <repo>/ work too)
emerge --info                       # system information
emerge --news                       # Arch Linux news

# Security
emerge neovim-git --aur --pkgbuild-view   # show/diff the PKGBUILD before building
emerge neovim-git --aur --edit            # edit the PKGBUILD first
emerge --scan neovim-git --aur            # audit only, no build
emerge --usepkgbuild ~/src/my-pkg         # local PKGBUILD through the scanner and sandbox
emerge --usepkg ./foo-1.0-1-x86_64.pkg.tar.zst  # prebuilt archive (pin + audit)

# Without installing
emerge -B neovim --aur              # only build the archives
emerge -f neovim --aur --outdistfiles ~/ae-out   # only download the whole tree into the chosen directory
emerge --usepkg ~/ae-out            # install all archives from a directory in one transaction

# Recovery
emerge --resume                     # resume an interrupted transaction
emerge --undo                       # undo the last install/removal/depclean
emerge --revdep-rebuild             # fix binaries with broken .so links
emerge --aelist                     # files left behind by AE Force
```

The full flag table is at the end of the page.

## Configuration

Everything is configured under `/etc/portage/`. The config is system-wide only and owned by root.

| Path | Purpose |
|------|---------|
| `make.conf` | Main config: default flags, colours, build flags, `FEATURES` |
| `package.mask`, `package.unmask` | What is never installed / exceptions |
| `package.protect`, `package.freeze` | What must not be removed / what must not be upgraded |
| `package.env`, `env/` | Per-package build flags |
| `make.profile` | Profile (a symlink) |
| `world`, `sets/` | Explicitly installed packages and custom sets |
| `repos.conf`, `ae-repos.conf` | Repository list (`FEATURES="repos"`) and `ae-*` sections for `pacman.conf` |
| `hooks/` | Custom alpm hooks |
| `resume.state`, `lastaction.state` | State for `--resume` and `--undo` |

Elsewhere: `/var/lib/aura-emerge/repos/` — local repositories, `/var/log/emerge.log` — operation log, `~/.cache/aura-emerge/` — caches (PKGBUILD view, devel, news, builds), `/var/db/repos/portage/profiles/` — profile database.

### make.conf

```bash
# Default flags. Spliced in before whatever you type (the command line always wins).
# Disable for one run: --ignore-default-opts.
EMERGE_DEFAULT_OPTS="--jobs=8 --tree --deep"

# Status-line colours (roles: ok, warn, error, info, special).
COLORS="ok=#FFB3AE warn=#E7BDB9 error=#FDB1AC info=#E7BDB9 special=#cba6f7"

# Build flags (see below)
CFLAGS="-march=native -O2 -pipe"
CXXFLAGS="$CFLAGS"
RUSTFLAGS="-C target-cpu=native"
MAKEFLAGS="-j$(nproc)"
NINJAFLAGS="-j$(nproc)"
OPTIONS=(strip docs !debug lto)
BUILDENV=(!distcc color check !sign)

# Switches for aura-emerge only
FEATURES="localrepo"
```

Syntax is bash-like: `KEY="..."`, `KEY='...'` (literal, no expansion) or `KEY=(a b c)`. `$(nproc)` and `$CFLAGS` work in double quotes and unquoted. The file is opened with `O_NOFOLLOW`: a symlink is rejected.

| Key | Purpose |
|-----|---------|
| `EMERGE_DEFAULT_OPTS` | Flags spliced into every run |
| `COLORS` | Palette for status output |
| `FEATURES` | aura-emerge switches |
| `EMERGE_FREEZE` | Same as `package.freeze`, on one line |
| `CFLAGS` `CXXFLAGS` `CPPFLAGS` `LDFLAGS` `RUSTFLAGS` `MAKEFLAGS` `NINJAFLAGS` `BUILDENV` `OPTIONS` | AUR/ABS build flags |

**`EMERGE_DEFAULT_OPTS`** accepts modifier flags only (`--ask`, `--verbose`, `--devel`, `--tree`, `--deep`, `--jobs=N`, `--jobsa=N`, `--searchdesc`, `--exclude=<atom>`, etc.), so you cannot turn every run into `--unmerge`. Conflicting pairs are rejected when both are set: `--aur`/`--abs`, `--aur`/`--repos`, `--no-sandbox`/`--unshare-net-build`, `--skippgp`/`--autopgp`, `--select`/`--deselect`. A config value that conflicts with the command line is dropped. `--abs` and `--repos` do **not** conflict with each other.

**`COLORS`** — `role=colour` tokens; a colour is `#RRGGBB` or a name (`red`, `bright-cyan`, …). The last mention of a role wins; unknown roles and colours are skipped with a warning.

**`FEATURES`** — for aura-emerge only, never translated into makepkg (set ccache, check, strip in `BUILDENV` / `OPTIONS`). `-name` turns a token off, the last mention wins. Tokens:

| Token | What it enables |
|-------|-----------------|
| `localrepo` | local repositories `ae-aur` / `ae-abs` / `ae-local` ([details](#local-repositories)) |
| `repos` | Portage-style `repos.conf` ([details](#reposconf)) |

Unknown tokens are read and shown in `--info` but do nothing. `emerge --info` prints the effective `FEATURES` and the values that override makepkg. [**=> make.conf example**](misc/make.conf)

#### Gentoo => aura-emerge

| Gentoo (`make.conf`) | aura-emerge (`make.conf`) |
|----------------------|---------------------------|
| `CFLAGS`, `CXXFLAGS`, `RUSTFLAGS` | the same |
| `MAKEOPTS="-j$(nproc)"` | `MAKEFLAGS="-j$(nproc)"` |
| `USE="lto"` | `OPTIONS=(... lto)` |
| `FEATURES="ccache"` | `BUILDENV=(... ccache)` |
| `FEATURES` (Portage functions) | `FEATURES` for AE only: `localrepo`, `repos` |

Arch has no USE flags, so `OPTIONS` partly plays their role. Reference: [GCC optimization options](https://gcc.gnu.org/onlinedocs/gcc/Optimize-Options.html), [rustc codegen options](https://doc.rust-lang.org/rustc/codegen-options/index.html).

### Build flags and makepkg.conf

Build flags are set only in `make.conf` (and per package in [`package.env`](#packageenv)). There is no need to edit `/etc/makepkg.conf` for aura-emerge.

Before every AUR/ABS build, emerge generates a temporary `makepkg.conf` itself: it first sources the system one, then overrides it with the keys from `make.conf`, and that file is passed to makepkg via `--config`. Hence:

- the system `/etc/makepkg.conf` is only a base for keys that `make.conf` does not have (`PKGEXT`, compression, `DLAGENTS`, etc.); on a key clash `make.conf` always wins;
- if `make.conf` has no build keys at all, makepkg is left untouched;
- inside the sandbox the user's `~/.config/pacman/makepkg.conf` has no effect (`$HOME` is empty); without the sandbox (`--no-sandbox`) makepkg sources it *after* the generated one, and a clashing key beats make.conf — emerge warns about it.

On security: values are escaped during generation (`"`, `` ` ``, `\`; a single-quoted value is never expanded), only tokens made of letters, digits and `!_-+.` are allowed in `BUILDENV` / `OPTIONS` (otherwise a warning, and the key stays as in the system file). The generated file lives in a private (`0700`) scratch directory outside the build tree so that `prepare()` / `build()` cannot rewrite it, and is deleted after the build.

### package.env

Per-package build flags. An entry is an atom plus one or more env files from `/etc/portage/env/`. `/etc/portage/package.env` is a file or a directory.

```
# /etc/portage/package.env/neovim
abs/neovim  lto.conf            # repo prefix and '*' work as in package.mask
aur/*-git   no-lto  fast        # several envs are applied in the given order
```

```bash
# /etc/portage/env/lto.conf  (make.conf syntax; only build keys are honoured)
CFLAGS="-march=znver3 -O2 -pipe"
MAKEFLAGS="-j16"
OPTIONS="strip docs !debug lto"
```

- **Layering:** `make.conf` => matching `package.env` entries => each env file in order. The layer that set the key last wins; a key is replaced as a whole, not merged. The build reports which layer won.
- **Matching:** by pkgbase or any pkgname.
- **Scope:** AUR/ABS builds only. `EMERGE_DEFAULT_OPTS` in an env file is ignored with a warning.
- **Errors:** a symlink, a malformed entry or a missing env file is skipped with a warning (file and line); the rest is applied.

### package.mask and package.unmask

`package.mask` lists packages this machine must never install. The check fires on every path: a request by name, a transitive AUR dependency, world provisioning, `--select`/`-R`/`-u`. A masked package is a hard stop, with no prompt and no flag to bypass it; the only way around is an entry in `package.unmask`.

`/etc/portage/package.mask` is a file or a directory (every regular file is read in name order; dotfiles are skipped; symlinks are rejected). One entry per line; `*`, a repository prefix and a trailing `# comment` are allowed.

```
ayugram-desktop-bin            # a specific package, from any source
aur/*-bin                      # never install -bin packages from the AUR
*-git                          # no devel packages on this machine
extra/nano                     # a specific package from a specific repo
```

The repository prefix is honoured wherever the source is known (`extra/nano` fires only for `extra`, `aur/foo-git` only for the AUR). Portage version atoms (`=cat/foo-1.2`, `>=cat/foo`) are not supported: matching is by name, and such entries are reported with a warning and skipped. A mask is permanent, unlike `--exclude`, which applies to one run.

`package.unmask` (same syntax) lists exceptions: a package covered by an entry passes even if `package.mask` or the active profile's mask blocks it. The root helper honours it too.

```
aur/some-git                   # allowed, even though *-git is masked above
```

### package.protect and package.freeze

**`package.protect`** lists packages that must not be removed (syntax as in `package.mask`). It applies to `-C`, `-c`, `--prune` and removals caused by conflicts. `--deselect` can still drop the name from world, but the package stays installed and depclean will not touch it. `@system` packages from the profile are protected the same way.

**`package.freeze`** (and the optional `EMERGE_FREEZE` in `make.conf`) keeps packages at the installed version during `-u`, including AUR/ABS rebuilds. Stricter than pacman's `IgnorePkg`: it also applies to a selective `emerge -u pkg`.

### Profiles

`/etc/portage/make.profile` is a symlink to a profile directory (Portage layout). `--list-profiles` shows the tree from `/var/db/repos/portage/profiles`; a `parent` of the form `repo:path` is looked up in `/var/db/repos/<repo>/profiles/`. A profile may contain:

- `parent` — chains are expanded all the way down;
- `package.profile` — `*` means the starter `@system` set (the old name `packages` is still read);
- `package.mask` — `-atom` or `!atom` lifts a mask inherited from a parent;
- `package.protect`, `make.defaults`.

`emerge @system` installs the profile's system set; every `package.profile` entry is protected from depclean just like world.

```bash
emerge --list-profiles
sudo emerge --set-profile 12
# or via the eselect link: eselect profile set 12
```

`/usr/bin/eselect` => `emerge` gives a minimal `eselect` (only `eselect profile list|set|show`). `--list-profiles` and `--set-profile` do not need it.

### world and sets

**`/etc/portage/world`** is the list of all explicitly installed packages. Entries are added on install, removed on removal, and can be edited by hand (one per line).

```
core/nano
extra/firefox
aur/ayugram-desktop-bin
abs/nano
local/my-pkg
chaotic-aur/visual-studio-code-bin
```

**Prefixes:** an official repository name, `aur/`, `abs/`, `local/` (`--usepkgbuild` and `--usepkg`), `Err/` (source unknown).

> **About `Err/`:** an installed package whose origin could not be determined, or an `aur/` / `abs/` entry that stopped resolving. `--regen-world` and `--regen-sets` keep a known prefix rather than falling back to `Err/`. During `emerge @world` such entries are only listed; you can install them with `--err-install`.

If one name in a batch install is not found, the rest is still installed; the missing ones are printed at the end and the exit code is non-zero.

**`@world` has two meanings:**

| Command | Action |
|---------|--------|
| `emerge @world` | *Provision:* installs what is in world but not yet installed. Keep world in your dotfiles, clone it onto a new machine, run this |
| `emerge -u` / `emerge -u @world` | *Upgrade:* upgrades everything installed (repos + AUR); does not read world |
| `emerge --sync` | Only refreshes databases (`-Sy` / `-Syy`) |

**Custom sets.** `/etc/portage/sets/<name>` (or `<name>.set`) is invoked as `@<name>`. One package per line, `#` comments allowed; a line `@other` nests another set (recursive, cycle-safe). Installing `@name` records `@name` in world, so the set's members are protected from depclean until you remove them from the set file. If both `game-kit` and `game-kit.set` exist, `@game-kit` is rejected as ambiguous.

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
emerge --batchinstall pkgs.txt        # same format, but the file can live anywhere and is not registered
```

`@world` and `@preserved-rebuild` are actions, so `-s @world` and `-s @preserved-rebuild` are errors.

## Working with packages

### Sources and prefixes

Without a prefix, emerge looks in the official repositories and falls back to the AUR. A prefix picks the source: `aur/<name>`, `abs/<name>`, `<repo>/<name>`. Prefixes work in commands, in `world`, in sets and in `-s`. `--aur`, `--repos`, `--abs` set the source for the whole run; AUR-only dependencies are built recursively.

### Parallel jobs and progress

`--jobs=N` installs up to N official-repo packages at once, `--jobsa=N` runs up to N AUR/ABS builds (defaults 1 and 1). `--load-average=X` holds back new builds while the one-minute load is above X (but never when nothing else is running). `--nospinner` removes the live Jobs line. Defaults are handy to keep in `EMERGE_DEFAULT_OPTS`.

```
>>> Emerging (3 of 12, 16%) noctalia-5.1.0-1.1
>>> Jobs: 26 of 26 complete  2m53s                         Load avg: 2.49, 3.11, 2.17
```

During a quiet build a heartbeat is printed every 30 s (pid, RSS: `still building … — process alive`); if the process is killed by a signal (OOM/SIGKILL), that is reported explicitly. If the helper rejects one package of a batch (for example, because it is masked), the batch is retried one package at a time so the rest still gets installed. `--debug` (or `AE_DEBUG=1`) shows compiler output live.

### Plan as a tree: `-t` and `--deep`

Shows the plan (with `-p` or before a real install) as a dependency tree. By default one level is shown; `--deep` goes to the bottom, `--deep=N` goes N levels.

```
[ebuild  N ] noctalia-5.1.0-1.1
[ebuild  N ] `-- libqalculate-5.12.0-1.1
[ebuild  N ]   `-- sdbus-cpp-2.3.1-1.1
```

A dependency that is not named in anyone's "Depends On" (resolved through `provides`, or deeper than `--deep`) is still shown, just without a parent. For a clean install via `--aur`, transitive AUR-only dependencies are not yet resolved at the plan step, so the tree is flat there.

### `--exclude` and `--keep-going`

`--exclude <atom>` (repeatable or comma-separated) excludes a package from the run: installs, `-u`, `@world`, `--depclean`/`--prune` (protects it from removal) and `--revdep-rebuild`.

`--keep-going` does not stop a batch operation (`@world`, `-u`, `--batchinstall`, several packages) at the first error: failures are remembered, a summary is printed at the end and the exit code is non-zero. The next `--resume` retries only what failed.

```bash
emerge -u --exclude linux --exclude aur/ayugram-desktop-bin
emerge -u --keep-going
emerge --resume
```

### Conflicts (blockers)

Installed packages that block an incoming one are shown before the install starts, as Portage blockers:

```
[ebuild  N    ] pipewire-jack-1:1.4.2-1
[conflict     ] jack2-1.9.22-1 ("jack2" is blocking pipewire-jack-1:1.4.2-1)
```

The check covers every source in the run: the repo plan (libalpm), AUR packages and their AUR dependencies, ABS packages (from `.SRCINFO`: `provides`/`conflicts`) and a bare `@world`. Sources are also checked **against each other**: if two packages in one run block each other (say, one from a repo and one from the AUR), it is shown as `[blocks]` and nothing is installed. Installed blockers need a single `Yes` (always asked); the packages that need them are listed first, and `package.protect` entries are never removed. Whatever the pre-check could not see (for example, no network for `.SRCINFO`) is still caught during the package's installation.

### Files without an owner: AE Force

If an install would overwrite a file no package owns, emerge asks `AE Force: Create .aenew for conflict files?`. "Yes" leaves your file in place and stores the package's version as `<file>.aenew`.

| Flag | What happens |
|------|--------------|
| `--aeforce` | no prompt; your file is moved to `<file>.aeforce`, the new one takes its place |
| `--noaefile` | no prompt; no `.ae*` files are left: old kept, new discarded (with `--aeforce`: old discarded, new kept) |

`<file>.aepark` is left after an interrupted install; if the name is taken, a `.N` suffix is added. `emerge --aelist` lists all such files, `emerge --aeclean [new|old|park|all]` removes them after a prompt (no argument = `all`).

### Hooks

libalpm runs every `*.hook` on install, removal, `-U` and `-Su`, just like pacman (depmod, DKMS, mkinitcpio/dracut, fontconfig and icon caches, `grub-mkconfig`, …). On a name clash the earlier directory wins:

```
/usr/share/libalpm/hooks/    hooks shipped by packages
/etc/pacman.d/hooks/         administrator hooks
/etc/portage/hooks/          aura-emerge's own hooks (alpm-hooks(5) format)
```

All three are always registered, even without `HookDir` in `pacman.conf`. Progress: `>>> Running pre-transaction hooks...` / `>>> (2 of 4) Install DKMS modules`.

### Updating

`emerge -u` upgrades the repos and the AUR (it does not read world). Two things a plain upgrade misses:

**Devel packages (`-git`, `-hg`, `-svn`, `-bzr`).** The AUR version changes only when the maintainer re-uploads the package, so `-u` often misses upstream changes. `-u --devel` (upgrade) and `--check-devel` (report only) close that gap: for each installed devel package they take the `git+` URL from the PKGBUILD, run `git ls-remote` (no clone needed) and compare with the hash from last time (`~/.cache/aura-emerge/devel.state`). Git only for now; `-hg`/`-svn`/`-bzr` are reported as "couldn't determine" rather than skipped silently.

**Broken library links.** Arch has no preserved-libs tracking, so there are two tools:

- `@preserved-rebuild` collects the *declared* dependencies of all installed packages (`pacman -Qi`), checks them with `pacman -T` and offers to reinstall the missing ones with `--asdeps` (so they do not end up in world via `--regen-world-from-explicit`);
- `--revdep-rebuild` reads ELF files (`DT_NEEDED`, `DT_RPATH`/`DT_RUNPATH`) and finds binaries linked against a `.so` that no longer exists (the classic breakage after an `openssl`/`icu`/`boost` upgrade). AUR/local packages are rebuilt through the sandbox; official-repo packages cannot be rebuilt here, so the missing library is looked up with `pacman -F` and the package that provides it is offered for installation. `--exclude` and the [mask](#packagemask-and-packageunmask) apply.

```bash
emerge -u --devel
emerge --check-devel
emerge @preserved-rebuild -p     # list only
emerge --revdep-rebuild -p       # scan only
emerge --revdep-rebuild          # scan, then ask before fixing
```

### Removing

`-C` removes the named packages, `-c` removes orphans, `--prune` removes everything that is not in world. `-C` prints what is `selected`, `protected` and `omitted`; then `-C` and `-c` wait 5 seconds (`Unmerging in: 5 4 3 2 1`, Ctrl-C aborts) unless `-a` has already asked or `-p` is set. After `-C`, the names are also dropped from world.

`-c` never removes a world entry, a member of a set recorded in world, a `package.protect` entry or an `@system` package from the profile. It reports how many were skipped; `--show-protected` lists them. With arguments, `-c` acts as a dependency-aware unmerge: only packages nobody needs are removed (`-cv` shows the reverse dependencies that block removal).

```bash
emerge -cp --show-protected
emerge -c
emerge --unmerge --ask linux-zen-headers
```

### Search and info

`-s` searches the official repos, then ABS, then the AUR. `-s aur/<name>` — AUR only, `-s abs/<name>` — ABS, `-s <repo>/<name>` — that repo first; `--repos` limits the search to official repos; `--search-all` shows the same name from every repo. Masked packages are tagged `[ Masked ]`. `-sv` adds the installed version and, for the AUR, the maintainer, votes, popularity, homepage, last-update date and license.

`emerge --info` prints the version (and the libalpm version), kernel, CPU/memory/swap, the effective `--jobs`/`--jobsa`, every repository in `pacman.conf` order with its mirror, values from `makepkg.conf` and `make.conf` (and which overrides won), `FEATURES`, the profile, mask/world sizes and merge-log totals.

`emerge --pkginfo nano` is an installed-package card (like a short `pacman -Qi`); with `-v` it also lists the files.

### Building without installing

| Flag | What it does |
|------|--------------|
| `-B`, `--buildpkgonly` | Archives only, no install: official => into `CacheDir`; AUR/ABS/local => built |
| `-f`, `--fetchonly` | Download only, the whole dependency tree: official => archives in `CacheDir`; AUR/ABS => clone + sources verified in the sandbox (`makepkg --verifysource`, into `SRCDEST`), for the package and its AUR/ABS dependencies; official dependencies as archives. No build, no install |
| `--outdistfiles <DIR>` | With `-B`/`-f`: copy the archives (with dependencies) into DIR; AUR/ABS checkouts are linked there |
| `-O`, `--nodeps` | Skip dependency resolution: official targets (`pacman -Sdd`); with `--usepkg` — a single archive (`-Udd`). AUR/ABS: not implemented, a warning is printed |
| `-o`, `--onlydeps` | Only the dependencies of official targets (not the targets themselves), implies `-1`. AUR/ABS: not implemented |

`-B` and `-f` do not use local binaries from `ae-*`. What you get can be deployed later: `emerge --usepkg <DIR>` installs every archive in a directory in one transaction, `emerge --usepkg -O <file>` installs a single archive without dependencies.

### `--resume` and `--undo`

`--resume` continues an interrupted transaction (`--skipfirst` skips the first package). `--undo` rolls back the last successful operation (one history step):

- **after an install** — removes only the packages that were new in that run and restores installed packages that were removed as blockers;
- **after a removal** — reinstalls from the recorded source (`Err/` and `abs/` are only listed);
- **after depclean** — reinstalls the removed orphans **as dependencies** (not added to world): repo packages from the repos, local builds through the AUR if it still has them. Versions are recorded: if the repos now have a different one, you are told; a package that is nowhere to be found is listed with a `pacman -U` line for the cached file. If some packages failed to come back, only they remain in the saved state and `--undo` can be run again;
- **after `-u`** — not supported (there is no version snapshot).

### Log and news

`/var/log/emerge.log` is append-only, one line per event; a failed write never breaks a merge. AUR/ABS packages are built one at a time, so each has its own timer; `pacman -S`/`-Syu` is a single transaction, so its line holds all atoms and the total time. `--info` shows the totals.

```
2026-09-21 08:51:44  MERGE    aur    noctalia-5.1.0-1.1        (12s)
2026-09-21 09:03:02  MERGE    repo   nano-8.0-1  vim-9.1-1     (4s)
2026-09-21 09:10:05  UNMERGE  -      old-package-2.0-1
```

`--log <PATH>` additionally writes a session log (command, emerge.log lines, full build output) — for bug reports; append-only.

`emerge --news` fetches the [Arch Linux news RSS feed](https://archlinux.org/feeds/news/); read state is kept in `~/.cache/aura-emerge/news.state` (no root needed).

```bash
emerge --news        # list, unread marked [N]
emerge --news 3      # read item 3
emerge --news all    # mark everything read
```

## Repositories

### Local repositories

With `FEATURES="localrepo"`, every package aura-emerge built from the AUR, ABS or a local PKGBUILD (or took from a `--usepkg` archive) is, after the audit and install, filed under `/var/lib/aura-emerge/repos/<repo>/` and the repository database is refreshed. The directories are owned by root (`0755`). It is an aurutils analogue: the aur / abs / local sources are kept apart at the libalpm database level.

| Repository | What it holds |
|------------|---------------|
| `ae-aur` | AUR builds |
| `ae-abs` | ABS builds (`--abs`) |
| `ae-local` | `--usepkgbuild` and `--usepkg` |

The repositories are filled either way, but pacman **sees** them only if they are listed in `/etc/pacman.conf` *after* the official ones (`core` / `extra` / `multilib`). Two ways:

```ini
# 1) include the shipped file (misc/ae-repos.conf => /etc/portage/ae-repos.conf)
Include = /etc/portage/ae-repos.conf
```

```ini
# 2) or write the sections yourself
[ae-aur]
SigLevel = Never
Server = file:///var/lib/aura-emerge/repos/ae-aur

[ae-abs]
SigLevel = Never
Server = file:///var/lib/aura-emerge/repos/ae-abs

[ae-local]
SigLevel = Never
Server = file:///var/lib/aura-emerge/repos/ae-local
```

What this gives you: `pacman -Qm` no longer lists these packages, `pacman -Si ae-aur/<pkg>` works, and `pacman -S` / `-Su` can reinstall them from the audited copy. On a name clash the first repository that has the name wins, so the `ae-*` sections must come last — the official ones keep priority. `SigLevel = Never` is deliberate: nothing here is signed, the packages were audited and sha256-pinned by emerge itself, and the directories are root-owned, so nobody else can swap a file.

What emerge does next:

- **Reuse:** a regular install of an AUR package whose version in `ae-aur` matches the current one in the AUR is installed from the local repo — no clone, no build. `--edit` and `--pkgbuild-view` force a rebuild; `-B` / `-f` do not use local binaries; an explicit `ae-aur/<name>` goes through the normal repo install.
- **Cleanup:** `-C`, `-c` and `--prune` also drop what they removed from `ae-*`. `emerge --localrepo-clean` removes every entry whose package is no longer installed (`-p` only lists, `-a` asks).
- **Empty databases** are created by the package's `post_install` / `post_upgrade` hook, by `emerge --sync` and by the first publish, so `pacman -Sy` does not stumble on a listed `file://` server with no packages.
- **A publish failure** is a warning, not a failed merge. If there are no `[ae-*]` sections, emerge prints a hint with the `Include` line once.

### repos.conf

A Portage-style repository list: `/etc/portage/repos.conf` (a file) or `/etc/portage/repos.conf/*.conf`. Enabled with `FEATURES="repos"`. Then `emerge --info` shows the order by priority (descending) and writes `/etc/portage/ae-generated-repos.conf`. Include it in pacman **instead of** listing the same repositories by hand: remove the corresponding `[repo]` blocks from `pacman.conf` and add

```ini
Include = /etc/portage/ae-generated-repos.conf
```

| Key | Meaning |
|-----|---------|
| `priority` | Higher = more preferred. On a tie, declaration order (a warning only, the numbers are not rewritten) |
| `sync-uri` | One or more URLs (`$repo` / `$arch`) |
| `include` | Path to a pacman mirrorlist (`Server = …`); compatible with rate-mirrors. Explicit `sync-uri` entries come first |
| `verify-signature` | A `SigLevel` string as is (`Never`, `Required DatabaseOptional`, …) |
| `location` | Optional (for `file://` — the repository directory) |

The shipped template is `misc/repos.conf` (sections commented out):

```ini
[g14]
priority = 55
sync-uri = https://arch.asus-linux.org
verify-signature = Required DatabaseOptional

[core]
priority = 50
include = /etc/pacman.d/mirrorlist
verify-signature = Required DatabaseOptional

[ae-aur]
priority = 10
sync-uri = file:///var/lib/aura-emerge/repos/ae-aur
verify-signature = Never
```

With `FEATURES="localrepo"`, missing `ae-aur` / `ae-abs` / `ae-local` sections are filled in with priorities 10 / 9 / 8.

## Security

### Privileges

The frontend never runs as root. If started as root (`sudo emerge …`, `doas`, `pkexec`, `su -`), it first starts the root helper, then permanently drops privileges to the invoking user (taken from `AE_USER`, `SUDO_UID`, `DOAS_USER`, `PKEXEC_UID` or the audit login uid; if none is found — it stops). Started as a regular user, the helper is launched through `sudo` on first need. The resolver, AUR/ABS builds, the sandbox and all caches run unprivileged; all libalpm work (install/remove/sync/upgrade/`-U`, signature checks) goes through the helper (`--ae-service`), which validates every request and enforces `package.mask` itself (`helper refused: … masked by …`).

**Helper start conditions:** the binary is owned by root and not group/world-writable (`install -m755 -o root -g root`), uid = 0, stdin/stdout are pipes (not a TTY), the parent process is alive. File conflicts on `-Su` / `-U` are resolved through AE Force instead of breaking the whole upgrade.

### PKGBUILD scanning

Before every AUR install, the `PKGBUILD` (and any `.install`) is fetched from cgit and scanned in two layers:

1. **AST** (`tree-sitter-bash`): `curl|sh`, `base64`/`xxd | sh`, `source <(curl …)`, `eval "$(curl …)"`, `sudo`/`pkexec`/`doas`, inline execution for python/perl/ruby/node/lua, `sh -c` with a suspicious payload, top-level command substitution, string-concatenation bypasses, and so on.
2. **Line heuristics:** `chmod 777`, bare IPv4 addresses, third-party package managers (`npm`/`pnpm`/`bun`/`yarn`/`pip`/`gem install …`, remote-only `cargo install`/`go install`), `bash <(curl …)`, `curl … | python3/perl/node/…`, unpacking straight into a shell (`gunzip`/`zstd -d`/`unzip -p`/`tar -xO | sh`), `/dev/tcp` and `/dev/udp`, heredocs fed to a shell, paste sites, `.onion`, anti-sandbox checks, known campaign IOCs (Atomic Arch, openconnect-sso, the 2018 `compromised.txt`, …).

Findings are `Suspicious` or `ConfirmedIoc`. A hit is not a hard block: you get the file and line, a cgit link and `Continue anyway? [y/N]`.

**Auxiliary files.** When the checkout is on disk (a build, or `--usepkgbuild <PATH> --scan`), the following are checked as well:

- the `Makefile` (`Makefile`/`makefile`/`GNUmakefile`) and shell scripts referenced by the PKGBUILD (`*.sh`, `*.bash`, `*.zsh`, `*.fish`) — line by line: `curl|sh`, base64/hex in shell, bare IPs and the rest of the shell IOCs;
- patches (`*.patch`, `*.diff`, `*.dpatch`, `*.pch`, or content that looks like a unified/git diff) — by headers and added lines: paths outside the source tree (absolute, `..`, `.git`), symlink creation, binary hunks, overly long added lines, network calls in build/script targets, and the same shell IOCs.

`--scan <pkg> --aur` sees only what cgit serves (`PKGBUILD` and `.install`).

### PKGBUILD review and local sources

- **`--pkgbuild-view`** shows the PKGBUILD right before the build and asks for confirmation: a diff against the last copy you were shown (cache in `~/.cache/aura-emerge/pkgbuild-view/`) or the whole file the first time. If you decline, it offers to open `$EDITOR` instead of just skipping the package. It works after `--edit`, so you see your own changes. It applies only to directly requested AUR/ABS packages — not to recursive dependencies and not to batch paths (`@world`, `-u`, `--undo`).
- **`--usepkgbuild <PATH>`** builds and installs a local directory with a PKGBUILD through the same pipeline as the AUR/ABS (scanner => sandbox) instead of a bare `makepkg -si`. `PATH` must contain a `PKGBUILD`; a missing `.SRCINFO` is generated. It combines with `--pkgbuild-view`, `-a`, `-1`, `--skippgp`, `--no-sandbox`, `--unshare-net-build`, `--skip-srcinfo-regen`, but not with packages/`@sets` or `--aur`/`--abs`. It is recorded in world as `local/` unless `-1` is given.
- **`--usepkg <FILE…|DIR>`** installs prebuilt `.pkg.tar.*` archives (makepkg output, a package from a file-based repo, a directory after `-f --outdistfiles`) with no PKGBUILD at all. Each archive is hashed, unpacked and audited, re-hashed right before the privileged install, and only then handed to libalpm (details in "Auditing prebuilt archives"). Recorded in world as `local/` unless `-1` is given. Several archives in one run form a single transaction; dependencies among them are resolved together.
- **`--scan`** audits the PKGBUILD (+ `.install`) without building or installing: for the AUR it takes the files from cgit, for a local directory — `--usepkgbuild <PATH> --scan`. The report is the same as before a build, without the "Continue anyway?" prompt. Non-zero exit on any finding, so it is script-friendly. Not yet available for `--abs`.

```bash
emerge neovim-git --aur --pkgbuild-view
emerge --usepkgbuild ~/src/my-pkg --pkgbuild-view --skippgp
emerge neovim-git --aur --scan
```

### Sandbox

**AUR and ABS** builds run `pkgver()` / `prepare()` / `build()` / `check()` / `package()` inside [`bwrap`](https://github.com/containers/bubblewrap). This is mandatory; the only way out is an explicit `--no-sandbox`. If `bubblewrap` is not installed, the build is rejected.

- an empty tmpfs over `/home` and `/run` (no real `$HOME`, no session D-Bus, ssh-agent or gpg-agent);
- `--clearenv` plus a small allowlist (`PATH`, `TERM`, `LANG`, the build destination directories, …);
- a copy of the GnuPG keyring with public keys only;
- PKGBUILD and `*.install` are re-mounted read-only;
- writes only to the build directory (+ the configured `PKGDEST` / `SRCDEST` / …);
- the network is shared by default, other namespaces are disabled;
- `fakeroot` in `package()` runs in a nested `bwrap` with uid 0;
- a seccomp denylist (ptrace, kexec, module load/unload, mount/umount, pivot_root, reboot, swapon/swapoff, iopl/ioperm, bpf, userfaultfd, …).

| Flag | Effect |
|------|--------|
| `--no-sandbox` | Disable bwrap; plain `makepkg -si` (also skips the confirmation of the no-audit fallback) |
| `--unshare-net-build` | Network only for download and `prepare()`; `build()`/`check()`/`package()` have none |

Dependency installation (`pacman -S`) and the final `pacman -U` run **outside** the sandbox — they do not execute PKGBUILD code.

### Auditing prebuilt archives

The third barrier before installation: a prebuilt `*.pkg.tar.*` (a build result or `--usepkg`) is inspected with `bsdtar`:

- setuid/setgid binaries;
- `.INSTALL` scripts (re-scanned with the PKGBUILD scanner);
- sensitive paths: alpm hooks, systemd units, sudoers, udev, tmpfiles.d, binfmt.d, pam.d, dbus system.d, NetworkManager dispatcher.d, environment.d, sysusers.d, `ld.so.preload`, and so on;
- path traversal and symlink names among the archive members.

On a TTY a confirmation is required even without `--ask`; in non-interactive mode — a warning and carry on. A failed `bsdtar` audit stops the install instead of installing without an audit.

### The one fallback that asks for confirmation

A PKGBUILD whose dependencies cannot be resolved statically (no `.SRCINFO` and a dynamic `depends=(…)`) is the only build path that ends in a bare `makepkg -si` with no sandbox and no audit. It never runs silently: interactively — an explicit Yes/No question for that package, with `--noconfirm` — a refusal (fail-closed). `--no-sandbox` skips this question: it is a conscious opt-out of the sandbox for the whole run.

### Further hardening

- Arch news text is sanitised (ANSI/bidi cannot reach the terminal or `news.state`).
- Config, mask, env and world files are opened with `O_NOFOLLOW`, so a symlink cannot be slipped in through a race.
- Rust dependencies are checked with [`cargo deny`](https://github.com/EmbarkStudios/cargo-deny) (`make deny`).

<details>
<summary><h3>Scanner check (after installation)</h3></summary>

```bash
git clone https://github.com/Undercat037/ae-scanner-checker.git
emerge --usepkgbuild ./ae-scanner-checker --scan
```
</details>

<details>
<summary><h2>Flags</h2></summary>

| Flag | Description |
|------|-------------|
| `-s`, `--search` | Search for packages |
| `-S`, `--searchdesc` | Search by description |
| `--search-all` | With `-s`: show the same name from every repo (like `pacman -Ss`) |
| `-sv` | Detailed package information |
| `-v`, `--verbose` | Verbose output / detailed info during search (`-sv`) |
| `-u`, `--update` | Full system upgrade (does not read world) |
| `--devel` | With `-u`: also rebuild `-git`/`-hg`/`-svn`/`-bzr` packages whose upstream moved ahead |
| `--check-devel` | Show devel packages with upstream changes (no rebuild) |
| `-C`, `--unmerge` | Remove packages (unmerge) |
| `-c`, `--depclean` | Remove orphans (depclean) |
| `--show-protected` | With `-c`: show orphans kept only because they are in world |
| `-p`, `--pretend` | Pretend (dry run) |
| `-a`, `--ask` | Ask before acting |
| `--sudoloop` | Keep the sudo timestamp warm (refreshed every 60 s) for the whole run |
| `-1`, `--oneshot` | Install as a dependency (does not go into world) |
| `-n`, `--noreplace` | Do not reinstall if already present (same or newer version) |
| `--with-optdeps` | Also install optional dependencies of official-repo packages |
| `--aur` | Force AUR only (AUR-only dependencies are always built recursively) |
| `--repos` | Official repositories only (leave the AUR alone) |
| `--abs` | Build from ABS (`pkgctl`) in the bwrap sandbox |
| `--no-sandbox` | Disable bwrap for AUR/ABS; plain makepkg with no isolation (also skips the fallback confirmation) |
| `--unshare-net-build` | No network during build()/check()/package() (the network stays on in `prepare()`) |
| `--skippgp` | Skip PGP checks during ABS builds (`--skippgpcheck`) |
| `--autopgp` | Auto-import missing PGP keys before an `--abs` build |
| `--edit` | Edit the PKGBUILD in `$EDITOR` before building (requested package only); regenerates `.SRCINFO` after saving |
| `--skip-srcinfo-regen` | With `--edit`: do not regenerate `.SRCINFO` |
| `--pkgbuild-view` | Show/diff the PKGBUILD and confirm before building (requested package only; on refusal offers `$EDITOR`) |
| `--usepkgbuild <PATH>` | Build and install a local directory with a PKGBUILD (scanner + bwrap); recorded in world as `local/` without `-1` |
| `--usepkg <FILE…\|DIR>` | Install local `.pkg.tar.*` archives through audit + helper `-U` in one transaction (dependencies among them are resolved together; a directory = every archive in it, e.g. the result of `-f --outdistfiles`); recorded in world as `local/` without `-1` |
| `-O` / `--nodeps` | With `--usepkg`: install exactly one archive file with no dependency check (`pacman -Udd`) |
| `--scan` | Audit PKGBUILD/.install only, no build/install; non-zero exit on findings (AUR or `--usepkgbuild`) |
| `--sync` | Synchronise package databases |
| `--refresh` | Force `-Syy` (with `--sync` or `-u`) |
| `--info` | System information, like Gentoo's `emerge --info` |
| `--pkginfo <pkg…>` | Details of an installed package (`-v` lists files) |
| `--news [N\|all]` | Arch Linux news: no argument — list, `N` — read an item, `all` — mark everything read |
| `--select` | Add to world without installing |
| `--deselect` | Remove from world without uninstalling |
| `--check-world` | Show world entries that are missing from the system or installed non-explicitly |
| `--sync-explicit` | Mark as dependencies the explicitly installed packages that are not in world, in a world `@set` or in `package.profile` (so `-c` can see them). Standalone: runs immediately (`-p` lists, `-a` asks). With an install/upgrade: after it completes successfully. Can go in `EMERGE_DEFAULT_OPTS` |
| `--prune` | Remove packages that are not in world |
| `--localrepo-clean` | Drop packages that are no longer on the system from `ae-aur` / `ae-abs` / `ae-local` (`FEATURES="localrepo"`; `-p` lists, `-a` asks) |
| `--regen` | Recreate the package metadata cache |
| `--regen-world` | Re-resolve repository prefixes in world |
| `--regen-sets <SET>` | Re-resolve prefixes in a custom set (`@game-kit` or `game-kit`) |
| `--regen-sort` | With `--regen-sets`: also sort the file (comments and grouping are lost) |
| `--regen-world-from-explicit` | Migration: add explicitly installed packages (`pacman -Qeq`) that are not in world, in a world `@set` or in `package.profile` to world (one-off). `-p` only lists, `-a` asks |
| `--err-install` | With `@world`: also install `Err/` entries (source unknown) |
| `--resume` | Resume an interrupted transaction |
| `--skipfirst` | With `--resume`: skip the first package |
| `--undo` | Undo the last install/removal/depclean (one step; `-u` is not rolled back) |
| `--clean-source-cache` | Clear the persistent VCS source cache and exit |
| `--batchinstall <FILE>` | Bulk install from a text list (same format as a custom set) |
| `--list-sets` | List available `@sets` (also used by completion) |
| `--exclude <ATOM>` | Exclude a package from this run (repeatable / comma-separated) |
| `--keep-going` | Do not stop a batch operation at the first error; failures go into the summary at the end |
| `--revdep-rebuild` | Rebuild packages whose binaries link against a missing library |
| `-t`, `--tree` | Show the plan as a dependency tree |
| `-D`, `--deep[=N]` | With `-t`: nest N levels, without a value — all of them |
| `--jobs <N>` | Max parallel official-repo installs (default 1) |
| `--jobsa <N>` | Max parallel AUR/ABS builds (default 1) |
| `--load-average <X>` | Do not start new builds while the one-minute load average is above X |
| `--nospinner` | No live Jobs line |
| `--color <y\|n\|auto>` | Force colour on or off |
| `--quiet-build=n` | Same as `--debug` (Gentoo spelling) |
| `--list-profiles` | List Portage-style profiles (like `eselect profile list`) |
| `--set-profile <N\|PATH>` | Set `/etc/portage/make.profile`: an index from `--list-profiles` or a path |
| `--aeforce` | AE Force: move files that block an install to `<file>.aeforce` without asking |
| `--noaefile` | AE Force: leave no `.ae*` files (keep the old one, discard the new; with `--aeforce`: discard the old one) |
| `--aelist` | List files left by AE Force (`.aenew` / `.aeforce` / `.aepark`) |
| `--aeclean [new\|old\|park\|all]` | Remove AE Force files after a prompt (no argument = `all`) |
| `--ignore-default-opts` | Ignore `EMERGE_DEFAULT_OPTS` from make.conf for this run |
| `--debug` | Show makepkg/compiler output live (also `AE_DEBUG=1`); hidden by default to keep the `>>>` lines readable, and when a build fails the saved log is still printed |
| `--log <PATH>` | Write a session log (command, emerge.log lines, full build output) for bug reports — no `| tee` needed; append-only: existing content is never truncated or overwritten |
| `-B` / `--buildpkgonly` | Archives only, no install (official => CacheDir; AUR/ABS/local => build) |
| `-f` / `--fetchonly` | Download only, the whole dependency tree: official => archives in CacheDir (with dependencies); AUR/ABS => clone + sources verified in the sandbox (`makepkg --verifysource`, into SRCDEST), for the package and its AUR/ABS dependencies, official dependencies as archives. No build, no install |
| `--outdistfiles <DIR>` | With `-B`/`-f`: copy the archives (with dependencies) into DIR; AUR/ABS checkouts are linked there |
| `-N`/`--newuse`, `-e`/`--emptytree`, `-q`, `-o`, `-l`, `-A`, `--onlydeps`, `--complete-graph`, `--changed-use`, `--backtrack`, `--columns`, `--newrepo`, `--reinstall`, `--with-bdeps`, `--metadata`, `--clean`, `--config`, `--noconfmem`, `--verbose-conflicts` | Gentoo compatibility: accepted, do nothing |
| `-h`, `--help` | Help |
| `-V`, `--version` | Version |

The `emerge(1)` man page is generated from the CLI (`emerge --gen-manpage`), so it cannot drift from `--help`.
</details>

## Author / License
Copyright (C) 2026 **Undercat037**
License GPL-3.0
