//! Phase 0.2: ek command ke time ka "context snapshot" capture karna.
//!
//! Kya capture hota hai:
//!   - OS + arch
//!   - relevant tool versions (command ke pehle word ke hisaab se: node/rustc/python..)
//!   - git state (branch, short HEAD, dirty?)
//!   - key files (.env presence, lockfile hashes, .nvmrc content)
//!   - env allowlist (sirf non-secret, build-affecting vars)
//!   - PATH entries
//!
//! Privacy: sirf allowlisted env vars store hote hain (secrets nahi). .env ka
//! content kabhi store nahi hota — sirf "present"/"absent".

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::process::Command;

/// Ek captured context snapshot (DB me jaane ke liye, sab JSON-as-string).
pub struct Snapshot {
    pub os: String,
    pub env_json: String,
    pub path_json: String,
    pub tool_versions: String,
    pub git_branch: Option<String>,
    pub git_head: Option<String>,
    pub git_dirty: Option<bool>,
    pub key_files: String,
}

/// Build-affecting, non-secret env vars. (Secrets — API keys, tokens — yaha NAHI.)
const ENV_ALLOWLIST: &[&str] = &[
    "NODE_ENV",
    "JAVA_HOME",
    "GOPATH",
    "GOROOT",
    "VIRTUAL_ENV",
    "CONDA_DEFAULT_ENV",
    "RUSTUP_TOOLCHAIN",
    "CARGO_HOME",
    "PYENV_VERSION",
    "ASDF_DIR",
    "LANG",
    "SHELL",
];

/// Files jinka presence/content build break/fix kar sakta hai.
const KEY_FILES: &[&str] = &[
    ".env",
    ".env.local",
    ".nvmrc",
    ".node-version",
    "package-lock.json",
    "yarn.lock",
    "pnpm-lock.yaml",
    "Cargo.lock",
    "go.mod",
    "go.sum",
    "requirements.txt",
    "poetry.lock",
    "Pipfile.lock",
    "Gemfile.lock",
    "composer.lock",
];

/// Har tool ka version kaise nikaale. (java/go `--version` use nahi karte!)
const VERSION_ARGS: &[(&str, &[&str])] = &[
    ("node", &["--version"]),
    ("npm", &["--version"]),
    ("python3", &["--version"]),
    ("rustc", &["--version"]),
    ("cargo", &["--version"]),
    ("go", &["version"]),
    ("java", &["-version"]),
    ("deno", &["--version"]),
    ("bun", &["--version"]),
    ("ruby", &["--version"]),
    ("php", &["--version"]),
    ("docker", &["--version"]),
];

/// Command ke pehle word se decide karo kaunse tools relevant hain.
fn relevant_tools(command: &str) -> &'static [&'static str] {
    let prog = command.split_whitespace().next().unwrap_or("");
    let prog = prog.rsplit(['/', '\\']).next().unwrap_or(prog);
    match prog {
        "node" | "npm" | "npx" | "yarn" | "pnpm" | "vite" | "next" | "nuxt" | "webpack"
        | "tsc" | "ts-node" | "jest" | "eslint" | "nest" => &["node", "npm"],
        "python" | "python3" | "pip" | "pip3" | "pytest" | "poetry" | "uv" => &["python3"],
        "cargo" | "rustc" | "rustup" | "clippy-driver" => &["rustc", "cargo"],
        "go" | "gofmt" => &["go"],
        "java" | "javac" | "gradle" | "mvn" | "kotlin" => &["java"],
        "deno" => &["deno"],
        "bun" => &["bun"],
        "ruby" | "gem" | "bundle" | "rails" | "rake" => &["ruby"],
        "php" | "composer" => &["php"],
        "docker" | "docker-compose" => &["docker"],
        _ => &[],
    }
}

/// `bin <args>` chalao cwd me, pehli non-empty line lautao (stdout ya stderr).
/// Sirf success (exit 0) pe — warna None (e.g. git rev-parse on unborn repo).
fn run_capture(bin: &str, args: &[&str], cwd: &str) -> Option<String> {
    let out = Command::new(bin).args(args).current_dir(cwd).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let text = if stdout.trim().is_empty() {
        String::from_utf8_lossy(&out.stderr).into_owned()
    } else {
        stdout.into_owned()
    };
    let first = text.lines().next()?.trim().to_string();
    (!first.is_empty()).then_some(first)
}

fn version_of(tool: &str, cwd: &str) -> Option<String> {
    let args = VERSION_ARGS.iter().find(|(t, _)| *t == tool).map(|(_, a)| *a)?;
    run_capture(tool, args, cwd)
}

/// File bytes ka chhota stable hash (lockfile change detect karne ke liye).
fn short_hash(bytes: &[u8]) -> String {
    let mut h = DefaultHasher::new();
    bytes.hash(&mut h);
    format!("{:08x}", h.finish() & 0xffff_ffff)
}

fn capture_key_files(cwd: &str) -> String {
    let mut map = serde_json::Map::new();
    for name in KEY_FILES {
        let path = Path::new(cwd).join(name);
        if !path.exists() {
            continue;
        }
        let value = if name.starts_with(".env") {
            // privacy: content kabhi nahi, sirf presence
            "present".to_string()
        } else if *name == ".nvmrc" || *name == ".node-version" {
            // version pin — content useful aur safe
            std::fs::read_to_string(&path)
                .map(|s| s.trim().to_string())
                .unwrap_or_else(|_| "present".to_string())
        } else {
            // lockfiles etc. — hash, taaki change pakda ja sake
            std::fs::read(&path)
                .map(|b| short_hash(&b))
                .unwrap_or_else(|_| "present".to_string())
        };
        map.insert(name.to_string(), serde_json::Value::String(value));
    }
    serde_json::Value::Object(map).to_string()
}

fn capture_git(cwd: &str) -> (Option<String>, Option<String>, Option<bool>) {
    // git repo hai ya nahi
    let inside = run_capture("git", &["rev-parse", "--is-inside-work-tree"], cwd)
        .map(|s| s == "true")
        .unwrap_or(false);
    if !inside {
        return (None, None, None);
    }
    // --show-current unborn repo pe bhi branch naam deta hai (e.g. "master"),
    // detached HEAD pe empty. rev-parse HEAD unborn pe fail (None) ho jaayega.
    let branch = run_capture("git", &["branch", "--show-current"], cwd)
        .filter(|s| !s.is_empty());
    let head = run_capture("git", &["rev-parse", "--short", "HEAD"], cwd);
    let dirty = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(cwd)
        .output()
        .ok()
        .map(|o| !o.stdout.is_empty());
    (branch, head, dirty)
}

/// Pura snapshot capture karo.
pub fn capture(cwd: &str, command: &str) -> Snapshot {
    let os = format!("{} {}", std::env::consts::OS, std::env::consts::ARCH);

    // env allowlist
    let mut env_map = serde_json::Map::new();
    for key in ENV_ALLOWLIST {
        if let Ok(val) = std::env::var(key) {
            env_map.insert(key.to_string(), serde_json::Value::String(val));
        }
    }
    let env_json = serde_json::Value::Object(env_map).to_string();

    // PATH entries
    let path_entries: Vec<String> = std::env::var_os("PATH")
        .map(|p| {
            std::env::split_paths(&p)
                .map(|x| x.to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    let path_json = serde_json::to_string(&path_entries).unwrap_or_else(|_| "[]".into());

    // tool versions (command-aware)
    let mut tv = serde_json::Map::new();
    for tool in relevant_tools(command) {
        if let Some(v) = version_of(tool, cwd) {
            tv.insert(tool.to_string(), serde_json::Value::String(v));
        }
    }
    let tool_versions = serde_json::Value::Object(tv).to_string();

    let (git_branch, git_head, git_dirty) = capture_git(cwd);
    let key_files = capture_key_files(cwd);

    Snapshot {
        os,
        env_json,
        path_json,
        tool_versions,
        git_branch,
        git_head,
        git_dirty,
        key_files,
    }
}
