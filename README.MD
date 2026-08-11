# aura-emerge
en | [ua](UA-README.MD)

> Gentoo-style `emerge` for Arch Linux - installs from official repos, the AUR, and ABS, scans PKGBUILDs for supply-chain red flags before building, and runs untrusted build steps inside a `bwrap` sandbox.

Manages packages via a `world.set` file — install once, track forever.

## Installation

### Makepkg
```bash
git clone https://github.com/Undercat037/aura-emerge
cd aura-emerge
makepkg -si
```
### AUR:
```bash
paru -S aura-emerge
```
### Manual:

```bash
git clone https://github.com/Undercat037/aura-emerge
cd aura-emerge
cargo build --release 
sudo cp -r target/release/aura-emerge /usr/bin/emerge
ln -s /usr/bin/emerge /usr/bin/portageq
emerge --gen-completions [your-shell] # /usr/share/bash-completion/completions/emerge , /usr/share/zsh/site-functions/_emerge , /usr/share/fish/vendor_completions.d/emerge.fish
emerge --gen-manpage > /usr/share/man/man1/emerge.1
```

**Requirements:** `git` and `curl` (AUR git clones + RPC/cgit lookups, the news feed).  
**Optional:** `devtools` for `--abs` support (`emerge devtools`, provides `pkgctl repo clone`), `gnupg` for automatic PGP key import (`--autopgp`), `bubblewrap` for sandboxed builds (see [Sandboxing](#sandboxing) — falls back to plain unsandboxed `makepkg` if it's missing).

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
emerge foo-git --aur --aur-deep    # also resolve and build AUR-only deps recursively (alias --aur-full)
emerge neovim --only-repos         # install only from official repos, never touch the AUR
emerge neovim --abs                # build from ABS (Arch Build System) source via pkgctl repo clone, sandboxed in bwrap
emerge neovim --abs --no-sandbox   # build from ABS unsandboxed (plain makepkg -si, no bwrap isolation)
emerge neovim --abs --skippgp      # build from ABS, skip PGP signature check
emerge neovim --abs --autopgp      # build from ABS, auto-import missing PGP keys, no prompt
emerge neovim --abs --edit         # build from ABS, edit PKGBUILD before building ($EDITOR), auto-regenerates .SRCINFO
emerge neovim-git --aur --edit     # install from AUR, edit PKGBUILD before building, auto-regenerates .SRCINFO
emerge neovim --aur --edit --off-src-regen  # edit PKGBUILD without regenerating .SRCINFO afterward
emerge -c                          # remove orphans (depclean)
emerge -C neovim                   # remove specific package (unmerge)
emerge @world                      # provision: install what's missing from world.set (new machine, etc.)
emerge @world --err-inst           # provision, also retrying Err/ (unresolved-source) entries via the normal install path
emerge -u @world                   # full system update (official + AUR) — does not read world.set
emerge -u @world --refresh         # full system update, force-refresh databases first (like pacman -Syy)
emerge --sync                      # sync package databases
emerge @game-kit                   # install a custom set from /etc/emerge/sets.d/game-kit.set
emerge @game-kit neovim            # sets and plain packages can be mixed
emerge --list-sets                 # list @world, @preserved-rebuild, and every custom set
emerge --info                      # show system info (like Gentoo's emerge --info)
emerge --news                      # list Arch Linux news, newest first, flags unread ones
emerge --news 3                    # read news item 3 in full, marks it read
emerge --news all                  # dismiss all news notifications
emerge -s neovim                   # search in official repos + AUR
emerge -s neovim --only-repos      # search official repos only, skip AUR
emerge --searchdesc neovim         # search by package description
emerge -sv neovim                  # detailed info
emerge -sv neovim-git --aur        # detailed info from AUR only
emerge -1 htop                     # install as dependency (no world.set)
emerge -n neovim                   # do not reinstall if already installed (--noreplace)
emerge -p neovim                   # pretend (dry run)
emerge -a neovim                   # ask before installing
emerge --select neovim             # add to world.set without installing
emerge --deselect neovim           # remove from world.set without unmerging
emerge --prune                     # remove packages not tracked in world.set
emerge --regen                     # regenerate package metadata cache
emerge --regen-world               # re-resolve repository prefixes in world.set
emerge --regen-sets @game-kit      # re-resolve repository prefixes for one custom set
emerge --regen-world-from-explicit # one-time migration: seed world.set from every explicitly-installed package
emerge --resume                    # resume last interrupted transaction
emerge --resume --skipfirst        # resume, skipping the first package
emerge --undo                      # undo the last install or unmerge
emerge --clean-source-cache        # delete the persistent VCS source cache (-git/-hg/-svn checkouts)
emerge @preserved-rebuild          # check installed packages for missing deps, offer to reinstall
emerge @preserved-rebuild -p       # pretend: just list missing dependencies
emerge @preserved-rebuild -a       # ask before each pacman prompt during reinstall
```

## Sandboxing

`--abs` builds run the PKGBUILD's `pkgver()`/`prepare()`/`build()`/`check()`/`package()` functions inside a [`bwrap`](https://github.com/containers/bubblewrap) namespace, on by default when `bubblewrap` is installed: fake `$HOME` (no SSH/GPG/browser data), nothing writable outside the build dir (+ configured `PKGDEST`/`SRCDEST`/etc.), network shared, everything else (user/IPC/PID/UTS/cgroup) torn down.

Second, independent layer on top of the [PKGBUILD scanner](#security-pkgbuild-scanning) — the scanner blocks *known*-bad patterns before anything runs, the sandbox limits the blast radius of whatever it missed. Dependency install and the final `pacman -U` stay outside the jail (no PKGBUILD code runs there).

`--no-sandbox` falls back to plain `makepkg -si`. AUR builds aren't sandboxed yet — only `--abs` is wrapped so far.

## @preserved-rebuild

There's no preserved-libs tracking in Arch, so `@preserved-rebuild` collects dependencies declared by each installed package (`pacman -Qi`), passes them deduplicated to `pacman -T`, and offers to reinstall any unsatisfied ones via `pacman -S --asdeps`. The `--asdeps` matters — otherwise they'd look "explicit" and could get pulled into world.set by a later `--regen-world-from-explicit`.

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

> **Note on `Err/`:** it's used both for a genuinely locally-built package with no traceable origin and for an `abs/`/`aur/` package that later became unresolvable (e.g. removed from AUR) — once collapsed to `Err/`, that distinction is gone for good. `--regen-world`/`--regen-sets` now preserve a known `abs/`/`aur/` prefix across a regen instead of downgrading it to `Err/` on the first re-resolution miss, so this only bites entries that were already `Err/` before that fix.

Installing several packages at once no longer aborts the whole batch if one of them can't be found anywhere (typo, removed from AUR, etc.) — the rest still install, and the missing name(s) are listed at the end with a non-zero exit code.

### `@world` has two distinct meanings

This is the Gentoo behavior that matters most, so it's worth spelling out:

- **`emerge @world` (bare, no `-u`)** — *provision*. Installs whatever `world.set` lists that isn't on this system yet; nothing already installed is touched. Each entry's prefix decides the source (official repo, AUR resolver + PKGBUILD scan, or `abs`/unresolved — listed but skipped, needs `--abs`/manual install). The "set up a new machine" operation: commit `world.set` to dotfiles, clone, run `emerge @world`. Named packages install first, e.g. `emerge nano @world`.
- **`emerge -u @world`** (or `emerge -u`) — *upgrade*. Upgrades everything already installed, repos + AUR. Doesn't touch `world.set`.
- **`emerge --sync`** — just refreshes databases (`-Sy`/`-Syy`).

## Custom sets

Beyond `@world` and `@preserved-rebuild`, you can define your own sets under `/etc/emerge/sets.d/`. A file `/etc/emerge/sets.d/game-kit.set` is invoked as `@game-kit`:

```
# /etc/emerge/sets.d/game-kit.set
steam
lutris
gamemode
aur/protonup-qt
```

One atom per line, blank lines and `#` comments ignored. Sets can be combined with each other and with plain package names in the same command:

```bash
emerge @game-kit @dev-tools neovim
```

`emerge --list-sets` prints every set currently available (`@world`, `@preserved-rebuild`, plus one line per `sets.d/*.set` file) — this is also what shell completion calls behind the scenes, so `emerge @<TAB>` offers real set names.

## --undo

Reverses the last successful install or unmerge. Only one step of history is kept, best-effort:

- **After an install:** removes only the packages that were new in that run (`pacman -R`, drops from world.set). Upgrades/reinstalls in the same batch are untouched.
- **After an unmerge:** reinstalls using each atom's recorded prefix (`Err/`/`abs/` entries are just listed). Can't restore an exact prior version if it's no longer current.
- **After `-u`:** not supported — no per-package version history and no snapshot mechanism of its own. Roll back manually via `pacman -Qi`/the pacman log/cache if needed.

```bash
emerge --undo
```

## News

`emerge --news` fetches the official [Arch Linux news feed](https://archlinux.org/feeds/news/) and lists the most recent items, newest first, flagging unread ones with `[N]` — the same idea as Gentoo's `eselect news`, since Arch has no `Sync` step where breaking-change news would otherwise surface. Read/unread state is kept per-user in `~/.cache/aura-emerge/news.state`, so it never needs root.

```bash
emerge --news        # list recent items, flag unread ones
emerge --news 3      # show item 3 in full, mark it read
emerge --news all    # dismiss every currently listed item
```

## Security: PKGBUILD scanning

Before every AUR install, aura-emerge fetches the raw `PKGBUILD` (and any `.install` hook) from the AUR cgit mirror and scans it before building, in two layers:

- **AST checks** (`tree-sitter-bash`) for the highest-value signals — RCE pipelines, `sudo`/`pkexec`/`doas` mid-build, `eval` on a curl/wget substitution, python inline-exec — parsed as a real syntax tree so trivial text-matching evasions don't slip through.
- **Line heuristics** as fallback/complement: `curl|sh` pipe-to-shell, base64/hex/openssl-decoded payloads, `chmod 777`, raw IPs, foreign package-manager calls (`npm install`, `pip install`…), paste-site fetches, `.onion` addresses, and known IOCs (package names/hashes) from disclosed AUR campaigns (e.g. "Atomic Arch" June 2026, the openconnect-sso wave Jul/Aug 2026, the 2018 `compromised.txt` marker).

Findings are tagged `Suspicious` (plausible but generic) or `ConfirmedIoc` (exact match to a known campaign, shown louder).

A hit doesn't block the install — it prints the file/line, a cgit link, and asks `Continue anyway? [y/N]`. A cheap tripwire, not a substitute for reading the PKGBUILD yourself. `--abs` builds also get the [`bwrap` sandbox](#sandboxing) as a second, independent layer.

## Flags

| Flag | Description |
|------|-------------|
| `-s` | Search packages |
| `--searchdesc` | Search by description |
| `-sv` | Detailed package info |
| `-u` | Full system update (does not read world.set; see `world.set` section) |
| `-C` | Remove specific packages (unmerge) |
| `-c` | Remove orphans (depclean) |
| `-p` | Pretend (dry run) |
| `-a` | Ask for confirmation |
| `-1` | Install as dep (oneshot) |
| `-v` | Verbose output |
| `-n` | Do not reinstall (noreplace) |
| `--aur` | Force AUR |
| `--aur-deep` (alias `--aur-full`) | Also resolve and build AUR-only dependencies recursively; implies `--aur` |
| `--only-repos` | Only official repos, never search/install from the AUR |
| `--abs` | Build from ABS source via `pkgctl repo clone`, sandboxed in `bwrap` by default |
| `--no-sandbox` | Disable the `bwrap` sandbox for `--abs` builds, falling back to plain `makepkg -si` |
| `--skippgp` | Skip PGP checks when building from ABS |
| `--autopgp` | Auto-import missing PGP keys from a trusted keyserver when building from ABS, without prompting |
| `--edit` | Edit PKGBUILD before building — opens `$EDITOR`, then auto-regenerates `.SRCINFO` from the edit |
| `--off-src-regen` | With `--edit`, skip the automatic `.SRCINFO` regeneration (old behavior) |
| `--sync` | Sync databases |
| `--refresh` | Force-refresh all databases even if up to date (`pacman -Syy`), used with `--sync` or `-u` |
| `--info` | Show system info, mirroring Gentoo's `emerge --info` |
| `--news [N\|all]` | List Arch Linux news (bare), read item N in full, or `all` to dismiss everything |
| `--check-news [N\|all]` | Full alias for `--news` |
| `--select` | Add to world.set (no install) |
| `--deselect` | Remove from world.set (no unmerge) |
| `--prune` | Remove packages not in world.set |
| `--regen` | Regenerate metadata cache |
| `--regen-world` | Re-resolve repo prefixes in world.set |
| `--regen-sets <SET>` | Re-resolve repo prefixes for one custom set |
| `--regen-sort` | With `--regen-sets`, also alphabetically re-sort the set file |
| `--regen-world-from-explicit` | One-time migration: seed world.set from every explicitly-installed package |
| `--err-inst` | With `@world` provisioning, also retry `Err/` (unresolved-source) entries via the normal install path |
| `--resume` | Resume interrupted transaction |
| `--skipfirst` | Skip first pkg when resuming |
| `--undo` | Undo the last install or unmerge (not full-system upgrades — see `--undo` section) |
| `--clean-source-cache` | Delete aura-emerge's persistent VCS source cache (`-git`/`-hg`/`-svn` checkouts) |
| `--list-sets` | List available `@world` / `@preserved-rebuild` / custom sets |
| `--with-bdeps` | Pull in build-time deps (compat) |
| `--verbose-conflicts` | Verbose slot conflicts (compat) |
| `-h` / `--help` | Show help |
| `-V` / `--version` | Show version |

A man page is installed as `emerge(1)` — `man emerge` for the full reference. It's generated straight from the CLI definition via [`clap_mangen`](https://github.com/clap-rs/clap/tree/master/clap_mangen) at package time (`emerge --gen-manpage`, same idea as `--gen-completions`), so it can't drift out of sync with `--help`.

## Demo

[vhs-emerge](repo-files/emerge.gif)

## Author

**Undercat037**