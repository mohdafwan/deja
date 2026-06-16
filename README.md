# Déjà

A terminal that diffs command *context* — solves **"worked yesterday, fails today"**.

Har successful command ka context snapshot save hota hai. Jab wahi command fail ho,
Déjà last successful run se diff dikha ke batata hai **kya badla** (culprit detection).

> Full design: [`../MVP_PLAN.md`](../MVP_PLAN.md) ·
> GUI + packaging: [`../PHASE1_GUI_PLAN.md`](../PHASE1_GUI_PLAN.md) ·
> Releasing: [`RELEASING.md`](RELEASING.md)

## Install (GUI terminal)

Single self-contained binary — koi extra setup / runtime dependency nahi (~5.8MB download).

| Platform | Kaise |
|----------|-------|
| 🐧 Linux | `deja-term-*-linux-gnu.tar.xz` download → extract → `./deja-term` |
| 🍎 macOS | `deja-term-*-apple-darwin.tar.xz` download → extract → run (pehli baar right-click → Open) |
| 🪟 Windows | `.msi` download → double-click install (SmartScreen pe "Run anyway") |
| Power users | `curl --proto '=https' -LsSf <release-url>/deja-term-installer.sh \| sh` |

> Releases GitHub Actions se ban-te hain (`git tag` pe). Dekho [`RELEASING.md`](RELEASING.md).

## Build from source

```bash
cargo run -p deja-term       # GUI terminal
cargo run -p deja            # CLI hook mode (existing terminals ke liye)
```

## Status: Phase 0.4 ✅ — polished MVP

Phase 0.4 polish: `deja why` (manual diff), snapshot dedup (DB bloat fix),
real duration capture, history me relative time.

Core flow (Phase 0.3):

Har command + exit code capture → har command ka **context snapshot** (OS, tool
versions, git state, key files, env allowlist, PATH) → SQLite me store. Aur jab
koi command **fail** ho jo pehle (same cwd me) chali thi, deja last-good run se
**diff dikha ke culprit batata hai**:

```
$ node build.js
⏪ deja: ye command last 2 ghante pehle chali thi (run #41). Tab se ye badla:

   tool  node       v18.0.0 → v20.0.0     ⚠️ likely cause
   file  .env       present → (absent)    ⚠️ likely cause
   git   HEAD       a3f9c21 → b7e2d40

💡 Sabse sambhavit: node (tool). Isse start kar.
```

(Next: 0.4 polish — `deja why`, config; 0.5 Windows; 1.0 GUI terminal.)

## Build

```bash
cargo build --release
```

## Use (Phase 0.1)

Shell hook lagao (apne `~/.bashrc` ya `~/.zshrc` me):

```bash
# bash
eval "$(deja init bash)"
# zsh
eval "$(deja init zsh)"
```

Ab jo bhi command chalega wo background me record hoga. Dekhne ke liye:

```bash
deja history          # recent commands + exit codes
deja history --limit 50
```

## Commands

| Command | Kya karta hai |
|---------|---------------|
| `deja init <bash\|zsh>` | shell hook script print karta hai (eval karo) |
| `deja record ...` | (internal) hook isse call karta hai — manually use mat karo |
| `deja history [--limit N]` | recent runs (time + duration + exit) dikhata hai |
| `deja show <id>` | ek run ka pura context snapshot dikhata hai |
| `deja why [command]` | command ka last-good vs latest diff dikhata hai (bina arg = last failed) |

## Data

History yaha store hoti hai: `~/.local/share/deja/history.db` (SQLite).

## Roadmap

- [x] **0.1** — shell hook + command/exit capture + SQLite
- [x] **0.2** — context snapshot (node/git/env/key-files) + `deja show`
- [x] **0.3** — diff + culprit ranking on failure ← **core value working**
- [x] **0.4** — `deja why`, snapshot dedup, duration, relative time
- [ ] **0.5** — Windows + PowerShell
- [ ] **1.0** — GUI terminal (egui)
