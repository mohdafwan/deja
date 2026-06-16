//! Déjà CLI — shell hook mode (init/record/history/show/why).
//! Core logic (db/snapshot/diff) `deja-core` lib se aata hai.

mod shell;

use anyhow::Result;
use clap::{Parser, Subcommand};
use deja_core::{db, diff, snapshot};

#[derive(Parser)]
#[command(name = "deja", version, about = "Terminal jo 'kal chala aaj nahi' solve karta hai")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Shell hook print karo. Use: eval "$(deja init bash)"
    Init {
        /// Kaunsa shell: bash | zsh
        #[arg(default_value = "bash")]
        shell: String,
    },

    /// (Internal) Ek command run record karo. Shell hook ye call karta hai.
    Record {
        #[arg(long)]
        command: String,
        #[arg(long)]
        exit: i64,
        #[arg(long)]
        cwd: String,
        #[arg(long)]
        started_at: i64,
        // -1 = unknown (Phase 0.1 me duration capture nahi). allow_hyphen_values
        // taaki clap "-1" ko flag na samjhe.
        #[arg(long, allow_hyphen_values = true, default_value_t = -1)]
        duration_ms: i64,
        /// Failure pe last-good run se diff print karo (hook ye fail pe pass karta hai).
        #[arg(long)]
        explain: bool,
    },

    /// Recent commands dekho (verify ke liye).
    History {
        #[arg(long, default_value_t = 20)]
        limit: i64,
    },

    /// Ek run ka pura context snapshot dekho (verify ke liye).
    Show {
        /// run id (deja history se)
        id: i64,
    },

    /// Kyun fail ho rahi hai? Command ka latest run vs last-good run ka diff dikhao.
    /// Bina command ke = is folder ki sabse recent failed command.
    Why {
        /// Command (jaise "npm run build"). Na do toh last failed command.
        command: Option<String>,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Init { shell } => {
            print!("{}", shell::init_script(&shell)?);
        }

        Command::Record {
            command,
            exit,
            cwd,
            started_at,
            duration_ms,
            explain,
        } => {
            let conn = db::open()?;
            // Phase 0.2: command ke time ka context snapshot capture karo.
            let snap = snapshot::capture(&cwd, &command);

            // Phase 0.3: failure pe last-good run se compare karke culprit dikhao.
            // (snapshot store karne se PEHLE dhundo, taaki current run match na ho.)
            if explain && exit != 0 {
                if let Some((good_run, good_snap)) =
                    db::last_good_snapshot(&conn, &command, &cwd)?
                {
                    let changes = diff::diff_snapshots(&good_snap, &snap);
                    diff::print_report(&good_run, &changes);
                }
            }

            let snapshot_id = db::insert_snapshot(&conn, &snap)?;
            db::insert_run(
                &conn,
                &db::Run {
                    command,
                    cwd,
                    exit_code: exit,
                    duration_ms,
                    started_at,
                },
                Some(snapshot_id),
            )?;
        }

        Command::History { limit } => {
            let conn = db::open()?;
            let rows = db::recent_runs(&conn, limit)?;
            if rows.is_empty() {
                println!(
                    "Abhi tak koi command record nahi hui. Hook lagaya? eval \"$(deja init bash)\""
                );
                return Ok(());
            }
            for r in rows.iter().rev() {
                let mark = if r.exit_code == 0 { "✓" } else { "✗" };
                let dur = if r.duration_ms >= 0 {
                    format!(" {}ms", r.duration_ms)
                } else {
                    String::new()
                };
                let when = diff::humanize_since(r.started_at);
                println!(
                    "{mark} [{}] {} (exit {}{}) {}  —  {}",
                    r.id, when, r.exit_code, dur, r.command, r.cwd
                );
            }
        }

        Command::Show { id } => {
            let conn = db::open()?;
            match db::get_run_detail(&conn, id)? {
                None => println!("Run #{id} nahi mila. `deja history` se id dekho."),
                Some(detail) => {
                    let r = &detail.run;
                    let mark = if r.exit_code == 0 { "✓" } else { "✗" };
                    println!("{mark} run #{}  (exit {})", r.id, r.exit_code);
                    println!("  command : {}", r.command);
                    println!("  cwd     : {}", r.cwd);
                    match &detail.snapshot {
                        None => println!("  snapshot: (none)"),
                        Some(s) => {
                            println!("  --- snapshot ---");
                            println!("  os      : {}", s.os);
                            let git = if s.git_branch.is_none()
                                && s.git_head.is_none()
                                && s.git_dirty.is_none()
                            {
                                "(not a git repo)".to_string()
                            } else {
                                let branch = s.git_branch.as_deref().unwrap_or("?");
                                let head = s.git_head.as_deref().unwrap_or("no commits");
                                let dirty = if s.git_dirty == Some(true) {
                                    " (dirty)"
                                } else {
                                    ""
                                };
                                format!("{branch} @ {head}{dirty}")
                            };
                            println!("  git     : {git}");
                            println!("  tools   : {}", s.tool_versions);
                            println!("  envs    : {}", s.env_json);
                            println!("  keyfiles: {}", s.key_files);
                            let path_count = serde_json::from_str::<Vec<String>>(&s.path_json)
                                .map(|v| v.len())
                                .unwrap_or(0);
                            println!("  path    : {path_count} entries");
                        }
                    }
                }
            }
        }

        Command::Why { command } => {
            let conn = db::open()?;
            let cwd = std::env::current_dir()?.to_string_lossy().into_owned();

            // Target run dhundo: diya hua command, ya last failed command.
            let target_id = match &command {
                Some(c) => db::most_recent_run_id(&conn, c, &cwd)?,
                None => db::most_recent_failed_run_id(&conn, &cwd)?,
            };

            let Some(tid) = target_id else {
                match &command {
                    Some(c) => println!("'{c}' ka is folder me koi record nahi mila."),
                    None => println!("Is folder me koi failed command record nahi hai. 🎉"),
                }
                return Ok(());
            };

            let detail = db::get_run_detail(&conn, tid)?.expect("run id abhi mila tha");
            let cmd = detail.run.command.clone();
            let Some(cur_snap) = detail.snapshot else {
                println!("'{cmd}' ka snapshot missing hai (purana run?).");
                return Ok(());
            };

            match db::last_good_snapshot(&conn, &cmd, &cwd)? {
                None => println!(
                    "'{cmd}' is folder me pehle kabhi successfully nahi chali — koi baseline nahi."
                ),
                Some((good_run, _good_snap)) if good_run.id == detail.run.id => {
                    println!("'{cmd}' ka last run successful tha ✓ — compare karne ko kuch nahi.");
                }
                Some((good_run, good_snap)) => {
                    eprintln!("\n🔍 deja why: {cmd}");
                    let changes = diff::diff_snapshots(&good_snap, &cur_snap);
                    diff::print_report(&good_run, &changes);
                }
            }
        }
    }
    Ok(())
}
