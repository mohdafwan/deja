# Déjà

A terminal that diffs command *context* — solves **"worked yesterday, fails today"**.

When a command fails, Déjà compares it against the last successful run and shows exactly **what changed** so you can fix it fast.

```
$ node build.js
⏪ deja: this command last ran 2 hours ago (run #41). Here's what changed since then:

   tool  node       v18.0.0 → v20.0.0     ⚠️ likely cause
   file  .env       present → (absent)    ⚠️ likely cause
   git   HEAD       a3f9c21 → b7e2d40

💡 Most likely cause: node (tool). Start there.
```

---

## Install

Single self-contained binary — no extra setup or runtime required (~5.8MB).

| Platform | How |
|----------|-----|
| 🐧 Linux | Download `deja-term-*-linux-gnu.tar.xz` → extract → `./deja-term` |
| 🍎 macOS | Download `deja-term-*-apple-darwin.tar.xz` → extract → run (first time: right-click → Open) |
| 🪟 Windows | Download `.msi` → double-click install (click "Run anyway" on SmartScreen) |
| Power users | `curl --proto '=https' -LsSf <release-url>/deja-term-installer.sh \| sh` |

---

## Usage

Add the shell hook to your `~/.bashrc` or `~/.zshrc`:

```bash
# bash
eval "$(deja init bash)"

# zsh
eval "$(deja init zsh)"
```

Every command you run will now be recorded in the background. That's it.

### Commands

| Command | Description |
|---------|-------------|
| `deja history` | Show recent commands with exit codes and timestamps |
| `deja history --limit 50` | Show last 50 commands |
| `deja show <id>` | Show the full context snapshot for a run |
| `deja why` | Diff the last failed command against its last successful run |
| `deja why <command>` | Diff a specific command |

### Data

History is stored locally at `~/.local/share/deja/history.db` (SQLite) — nothing is sent anywhere.
