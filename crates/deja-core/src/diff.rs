//! Phase 0.3: do snapshots ka diff + culprit ranking + report.
//!
//! Jab koi command fail ho jo pehle (same cwd me) chali thi, hum last-good
//! snapshot aur current snapshot compare karke "kya badla" nikaalte hain,
//! culprit-score se rank karte hain, aur top changes dikhate hain.

use crate::db::RunRow;
use crate::snapshot::Snapshot;
use serde_json::{Map, Value};

// Culprit scores — jitna zyada, utna sambhavit wajah.
const TOOL_VERSION: u32 = 100; // node/rustc/python version badla = #1 reason
const KEYFILE_MISSING: u32 = 95; // .env/lockfile gayab
const OS_CHANGED: u32 = 85; // machine switch (rare par bada)
const NVMRC_CHANGED: u32 = 90; // version pin badla
const LOCKFILE_CHANGED: u32 = 70; // dep update
const KEYFILE_ADDED: u32 = 55; // nayi file aayi
const ENV_CHANGED: u32 = 55; // config drift
const PATH_CHANGED: u32 = 50; // tool resolution badla
const GIT_HEAD: u32 = 45; // code aage badha
const GIT_BRANCH: u32 = 45; // doosri branch
const GIT_DIRTY: u32 = 20; // uncommitted changes

/// Score itna ya zyada → "likely cause" tag.
const LIKELY_THRESHOLD: u32 = 80;

pub struct Change {
    pub category: &'static str,
    pub key: String,
    pub before: String,
    pub after: String,
    pub score: u32,
}

fn parse_obj(s: &str) -> Map<String, Value> {
    serde_json::from_str(s).unwrap_or_default()
}

fn val_str(v: &Value) -> String {
    v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string())
}

/// Dono maps ke saare keys (sorted, unique).
fn union_keys(a: &Map<String, Value>, b: &Map<String, Value>) -> Vec<String> {
    let mut keys: Vec<String> = a.keys().chain(b.keys()).cloned().collect();
    keys.sort();
    keys.dedup();
    keys
}

fn show(v: &Option<String>) -> String {
    v.clone().unwrap_or_else(|| "(absent)".to_string())
}

fn show_git(v: &Option<String>) -> String {
    v.clone().unwrap_or_else(|| "(none)".to_string())
}

/// old (last good) vs new (current/failed) — ranked changes.
pub fn diff_snapshots(old: &Snapshot, new: &Snapshot) -> Vec<Change> {
    let mut changes = Vec::new();

    if old.os != new.os {
        changes.push(Change {
            category: "os",
            key: "os".into(),
            before: old.os.clone(),
            after: new.os.clone(),
            score: OS_CHANGED,
        });
    }

    // tool versions
    let (ot, nt) = (parse_obj(&old.tool_versions), parse_obj(&new.tool_versions));
    for key in union_keys(&ot, &nt) {
        let b = ot.get(&key).map(val_str);
        let a = nt.get(&key).map(val_str);
        if b != a {
            changes.push(Change {
                category: "tool",
                key,
                before: show(&b),
                after: show(&a),
                score: TOOL_VERSION,
            });
        }
    }

    // key files
    let (ok, nk) = (parse_obj(&old.key_files), parse_obj(&new.key_files));
    for key in union_keys(&ok, &nk) {
        let b = ok.get(&key).map(val_str);
        let a = nk.get(&key).map(val_str);
        if b != a {
            let score = if a.is_none() {
                KEYFILE_MISSING
            } else if b.is_none() {
                KEYFILE_ADDED
            } else if key == ".nvmrc" || key == ".node-version" {
                NVMRC_CHANGED
            } else {
                LOCKFILE_CHANGED
            };
            changes.push(Change {
                category: "file",
                key,
                before: show(&b),
                after: show(&a),
                score,
            });
        }
    }

    // env allowlist
    let (oe, ne) = (parse_obj(&old.env_json), parse_obj(&new.env_json));
    for key in union_keys(&oe, &ne) {
        let b = oe.get(&key).map(val_str);
        let a = ne.get(&key).map(val_str);
        if b != a {
            changes.push(Change {
                category: "env",
                key,
                before: show(&b),
                after: show(&a),
                score: ENV_CHANGED,
            });
        }
    }

    // git
    if old.git_head != new.git_head {
        changes.push(Change {
            category: "git",
            key: "HEAD".into(),
            before: show_git(&old.git_head),
            after: show_git(&new.git_head),
            score: GIT_HEAD,
        });
    }
    if old.git_branch != new.git_branch {
        changes.push(Change {
            category: "git",
            key: "branch".into(),
            before: show_git(&old.git_branch),
            after: show_git(&new.git_branch),
            score: GIT_BRANCH,
        });
    }
    if old.git_dirty != new.git_dirty {
        let f = |d: Option<bool>| match d {
            Some(true) => "dirty".to_string(),
            Some(false) => "clean".to_string(),
            None => "(none)".to_string(),
        };
        changes.push(Change {
            category: "git",
            key: "dirty".into(),
            before: f(old.git_dirty),
            after: f(new.git_dirty),
            score: GIT_DIRTY,
        });
    }

    // PATH (count added/removed)
    let op: Vec<String> = serde_json::from_str(&old.path_json).unwrap_or_default();
    let np: Vec<String> = serde_json::from_str(&new.path_json).unwrap_or_default();
    if op != np {
        let removed = op.iter().filter(|x| !np.contains(x)).count();
        let added = np.iter().filter(|x| !op.contains(x)).count();
        if removed > 0 || added > 0 {
            changes.push(Change {
                category: "path",
                key: "PATH".into(),
                before: format!("{} entries", op.len()),
                after: format!("{} entries (+{added} -{removed})", np.len()),
                score: PATH_CHANGED,
            });
        }
    }

    changes.sort_by(|a, b| b.score.cmp(&a.score));
    changes
}

pub fn humanize_since(ts: i64) -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(ts);
    let secs = (now - ts).max(0);
    if secs < 90 {
        "abhi-abhi".to_string()
    } else if secs < 3600 {
        format!("{} min pehle", secs / 60)
    } else if secs < 86400 {
        format!("{} ghante pehle", secs / 3600)
    } else {
        format!("{} din pehle", secs / 86400)
    }
}

/// Failure report stderr pe print karo.
pub fn print_report(good_run: &RunRow, changes: &[Change]) {
    if changes.is_empty() {
        eprintln!(
            "\n⏪ deja: ye command pehle bhi chali thi (run #{}), par environment me kuch nahi badla.\n   Issue shaayad code ya arguments me hai.\n",
            good_run.id
        );
        return;
    }

    eprintln!(
        "\n⏪ deja: ye command last {} chali thi (run #{}). Tab se ye badla:\n",
        humanize_since(good_run.started_at),
        good_run.id
    );

    const SHOWN: usize = 5;
    for c in changes.iter().take(SHOWN) {
        let tag = if c.score >= LIKELY_THRESHOLD {
            "   ⚠️ likely cause"
        } else {
            ""
        };
        eprintln!(
            "   {:<5} {:<16} {}  →  {}{}",
            c.category, c.key, c.before, c.after, tag
        );
    }
    if changes.len() > SHOWN {
        eprintln!("   … +{} aur changes", changes.len() - SHOWN);
    }

    if let Some(top) = changes.first() {
        if top.score >= LIKELY_THRESHOLD {
            eprintln!(
                "\n💡 Sabse sambhavit: {} ({}). Isse start kar.\n",
                top.key, top.category
            );
        } else {
            eprintln!();
        }
    }
}
