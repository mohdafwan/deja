# Releasing Déjà

Distribution `cargo-dist` (`dist`) se hota hai — ek `git tag` se teeno platforms
ke installers ban ke GitHub Release pe chadh jaate hain.

## Kya ban-ta hai (har release pe)

| Platform | Artifact | Install |
|----------|----------|---------|
| 🐧 Linux x64 | `deja-term-x86_64-unknown-linux-gnu.tar.xz` | extract → `./deja-term` |
| 🍎 macOS arm64 | `deja-term-aarch64-apple-darwin.tar.xz` | extract → run |
| 🍎 macOS x64 | `deja-term-x86_64-apple-darwin.tar.xz` | extract → run |
| 🪟 Windows x64 | `deja-term-x86_64-pc-windows-msvc.zip` + **`.msi`** | MSI double-click install |
| All (Unix) | `deja-term-installer.sh` | `curl -LsSf <url> \| sh` |
| All (Win) | `deja-term-installer.ps1` | `irm <url> \| iex` |

Har artifact ke saath SHA256 checksum bhi.

> **Download size:** Linux tarball ~5.8MB (binary 18MB, xz-compressed). Hyper (~200MB)
> se bahut halka — kyunki egui single binary hai, koi Chromium/webview bundle nahi.

## Release process

1. **Repo GitHub pe ho** (abhi config me `github.com/af3an/deja` — apna actual repo
   URL `Cargo.toml` ke `[workspace.package] repository` me set karo).
2. Version bump: `Cargo.toml` me `[workspace.package] version`.
3. Tag + push:
   ```bash
   git tag v0.1.0
   git push origin v0.1.0
   ```
4. GitHub Actions (`.github/workflows/release.yml`) khud:
   - har platform pe build kare (mac/win/linux runners)
   - archives + installers + checksums banaye
   - ek GitHub Release create kare sab artifacts ke saath

Bas. Ek tag = teeno OS ke installers ready.

## Local testing (ek platform)

```bash
dist plan                                   # kya banega dekho
dist build --artifacts=local --target=x86_64-unknown-linux-gnu
ls target/distrib/                          # bana hua artifact
```

## Config kahan hai
- `[workspace.metadata.dist]` in root `Cargo.toml` — targets, installers, CI.
- `.github/workflows/release.yml` — auto-generated (haath se mat edit karo;
  config badal ke `dist generate` chalao).
- `crates/deja-term/wix/main.wxs` — Windows MSI definition (auto-generated).
- `crates/deja-cli` me `[package.metadata.dist] dist = false` — CLI release me nahi
  jaata (GUI main product hai).

## Abhi ki limitations + agle enhancements

| Cheez | Status | Note |
|-------|--------|------|
| Linux portability | tar.xz raw binary | desktop Linux pe system libs (libxkbcommon, libGL) usually present. **AppImage** se guaranteed-portable single file banega → `cargo-appimage` ya `linuxdeploy` add karo |
| macOS `.dmg` + Gatekeeper | abhi tar.xz | bina notarization ke user ko right-click→Open karna padega. Apple Developer ($99/yr) → `dist` me signing/notarization hooks |
| Windows SmartScreen | unsigned | "Run anyway" se chalega. Baad me code-signing cert |
| MSI Start-menu shortcut | basic MSI | cargo-dist MSI PATH add karta hai; shortcut custom WiX se |
| Auto-update | nahi | `install-updater` config / `axoupdater` se add ho sakta |
| Linux `.deb`/`.rpm` | nahi | `cargo-deb` se add ho sakta apt/dnf users ke liye |
| App icon | nahi | per-platform icon (.ico/.icns) add karna |
