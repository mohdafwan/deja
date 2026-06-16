# Déjà

A terminal that diffs command *context* — solves **"worked yesterday, fails today"**.

Every successful command saves a context snapshot. When the same command fails,
Déjà diffs against the last successful run to show **what changed** (culprit detection).

> Full design: [`../MVP_PLAN.md`](../MVP_PLAN.md) ·
> GUI + packaging: [`../PHASE1_GUI_PLAN.md`](../PHASE1_GUI_PLAN.md) ·
> Releasing: [`RELEASING.md`](RELEASING.md)

## Install (GUI terminal)

Single self-contained binary — no extra setup / runtime dependency required (~5.8MB download).

| Platform | How |
|----------|-----|
| 🐧 Linux | Download `deja-term-*-linux-gnu.tar.xz` → extract → `./deja-term` |
| 🍎 macOS | Download `deja-term-*-apple-darwin.tar.xz` → extract → run (first time: right-click → Open) |
| 🪟 Windows | Download `.msi` → double-click install (click "Run anyway" on SmartScreen) |
| Power users | `curl --proto '=https' -LsSf <release-url>/deja-term-installer.sh \| sh` |

> Releases are built via GitHub Actions (on `git tag`). See [`RELEASING.md`](RELEASING.md).

## Build from source

```bash
cargo run -p deja-term       # GUI terminal
cargo run -p deja            # CLI hook mode (for existing terminals)
```

## Status: Phase 0.4 ✅ — Polished MVP

Phase 0.4 polish: `deja why` (manual diff), snapshot dedup (DB bloat fix),
real duration capture, relative time in history.

Core flow (Phase 0.3):

Every command + exit code is captured → a **context snapshot** is taken for each command (OS, tool
versions, git state, key files, env allowlist, PATH) → stored in SQLite. When a
command **fails** that previously succeeded (in the same cwd), Déjà diffs against the last good run
and **highlights the culprit**:

```
$ node build.js
⏪ deja: this command last ran 2 hours ago (run #41). Here's what changed since then:

   tool  node       v18.0.0 → v20.0.0     ⚠️ likely cause
   file  .env       present → (absent)    ⚠️ likely cause
   git   HEAD       a3f9c21 → b7e2d40

💡 Most likely cause: node (tool). Start there.
```

(Next: 0.4 polish — `deja why`, config; 0.5 Windows; 1.0 GUI terminal.)

## Build

```bash
cargo build --release
```

## Usage (Phase 0.1)

Add the shell hook to your `~/.bashrc` or `~/.zshrc`:

```bash
# bash
eval "$(deja init bash)"
# zsh
eval "$(deja init zsh)"
```

Every command you run will now be recorded in the background. To view history:

```bash
deja history          # recent commands + exit codes
deja history --limit 50
```

## Commands

| Command | Description |
|---------|-------------|
| `deja init <bash\|zsh>` | Prints the shell hook script (pipe to eval) |
| `deja record ...` | (internal) Called by the hook — do not use manually |
| `deja history [--limit N]` | Shows recent runs (time + duration + exit code) |
| `deja show <id>` | Shows the full context snapshot for a single run |
| `deja why [command]` | Shows a last-good vs latest diff for a command (no arg = last failed) |

## Data

History is stored at: `~/.local/share/deja/history.db` (SQLite).

## Roadmap

- [x] **0.1** — shell hook + command/exit capture + SQLite
- [x] **0.2** — context snapshot (node/git/env/key-files) + `deja show`
- [x] **0.3** — diff + culprit ranking on failure ← **core value working**
- [x] **0.4** — `deja why`, snapshot dedup, duration, relative time
- [ ] **0.5** — Windows + PowerShell
- [ ] **1.0** — GUI terminal (egui)
