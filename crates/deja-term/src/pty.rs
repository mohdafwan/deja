//! Cross-platform PTY — shell spawn karta hai aur output ek channel pe bhejta hai.

use anyhow::Result;
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use std::io::{Read, Write};
use std::sync::mpsc::{channel, Receiver};

pub struct Pty {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    _child: Box<dyn portable_pty::Child + Send + Sync>,
}

impl Pty {
    /// Shell spawn karo. `on_data` har baar naya output aane pe call hota hai
    /// (egui repaint trigger karne ke liye). Returns (Pty, output receiver).
    pub fn spawn(
        rows: u16,
        cols: u16,
        on_data: impl Fn() + Send + 'static,
    ) -> Result<(Pty, Receiver<Vec<u8>>)> {
        let pty_system = native_pty_system();
        let pair = pty_system.openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;

        let shell = default_shell();
        let mut cmd = CommandBuilder::new(&shell);
        cmd.env("TERM", "xterm-256color");
        if let Ok(cwd) = std::env::current_dir() {
            cmd.cwd(cwd);
        }
        // bash ke liye shell-integration auto-inject (OSC 133 command markers).
        // User ko kuch setup nahi karna — hum apni rcfile ke saath bash spawn karte hain.
        if shell.ends_with("bash") {
            if let Ok(path) = write_bash_rcfile() {
                cmd.arg("--rcfile");
                cmd.arg(&path);
                cmd.arg("-i");
            }
        } else if shell.ends_with("zsh") {
            // zsh: ZDOTDIR ek temp dir pe set karo jisme hamari .zshrc ho
            if let Ok(dir) = write_zsh_zdotdir() {
                cmd.env("ZDOTDIR", &dir);
            }
        }

        let child = pair.slave.spawn_command(cmd)?;
        drop(pair.slave); // slave band karo taaki EOF sahi mile

        let mut reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;

        let (tx, rx) = channel::<Vec<u8>>();
        std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if tx.send(buf[..n].to_vec()).is_err() {
                            break;
                        }
                        on_data();
                    }
                }
            }
        });

        Ok((
            Pty {
                master: pair.master,
                writer,
                _child: child,
            },
            rx,
        ))
    }

    pub fn write(&mut self, data: &[u8]) {
        let _ = self.writer.write_all(data);
        let _ = self.writer.flush();
    }

    pub fn resize(&self, rows: u16, cols: u16) {
        let _ = self.master.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        });
    }
}

fn default_shell() -> String {
    if cfg!(windows) {
        std::env::var("COMSPEC").unwrap_or_else(|_| "powershell.exe".to_string())
    } else {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string())
    }
}

/// Bash shell-integration ek temp rcfile me likho aur uska path lautao.
/// User ki ~/.bashrc bhi source hoti hai, fir hamare OSC 133 markers add hote hain.
fn write_bash_rcfile() -> Result<std::path::PathBuf> {
    let path = std::env::temp_dir().join(format!("deja-bashrc-{}.sh", std::process::id()));
    std::fs::write(&path, BASH_INTEGRATION)?;
    Ok(path)
}

/// zsh ke liye temp ZDOTDIR banao jisme .zshrc ho (user ka zshrc + hamare markers).
fn write_zsh_zdotdir() -> Result<std::path::PathBuf> {
    let dir = std::env::temp_dir().join(format!("deja-zdotdir-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join(".zshrc"), ZSH_INTEGRATION)?;
    Ok(dir)
}

const ZSH_INTEGRATION: &str = r#"
# Déjà GUI terminal shell integration (auto-injected)
[ -f "$HOME/.zshrc" ] && source "$HOME/.zshrc"

__deja_b64() { printf '%s' "$1" | base64 | tr -d '\n'; }
__deja_preexec() { __DEJA_CMD="$1"; printf '\033]133;C\007'; }
__deja_precmd() {
  local ec=$?
  if [ -n "$__DEJA_CMD" ]; then
    printf '\033]133;D;%s;%s;%s\007' "$ec" "$(__deja_b64 "$__DEJA_CMD")" "$(__deja_b64 "$PWD")"
    __DEJA_CMD=""
  fi
  printf '\033]133;A;%s;%s\007' "$(__deja_b64 "$PWD")" \
    "$(__deja_b64 "$(git branch --show-current 2>/dev/null)")"
}
autoload -Uz add-zsh-hook
add-zsh-hook preexec __deja_preexec
add-zsh-hook precmd __deja_precmd
"#;

const BASH_INTEGRATION: &str = r#"
# Déjà GUI terminal shell integration (auto-injected)
if [ -f "$HOME/.bashrc" ]; then . "$HOME/.bashrc"; fi

__deja_b64() { printf '%s' "$1" | base64 | tr -d '\n'; }
__deja_osc() {
  local ec=$?
  local hnum cmd
  read -r hnum cmd <<< "$(HISTTIMEFORMAT='' history 1)"
  # D = command complete (pehla prompt skip — koi command nahi chali abhi tak)
  if [ -n "$__DEJA_READY" ] && [ -n "$cmd" ] && [ "$hnum" != "$__DEJA_LAST" ]; then
    __DEJA_LAST="$hnum"
    printf '\033]133;D;%s;%s;%s\007' "$ec" "$(__deja_b64 "$cmd")" "$(__deja_b64 "$PWD")"
  fi
  __DEJA_READY=1
  # A = naya block start, path + git branch ke saath
  printf '\033]133;A;%s;%s\007' "$(__deja_b64 "$PWD")" \
    "$(__deja_b64 "$(git branch --show-current 2>/dev/null)")"
}
# C = command execute hone se theek pehle (output start) — PS0 ANSI-C escapes
case "$PS0" in
  *'133;C'*) ;;
  *) PS0=$'\e]133;C\a'"$PS0" ;;
esac
case ";${PROMPT_COMMAND};" in
  *";__deja_osc;"*) ;;
  *) PROMPT_COMMAND="__deja_osc${PROMPT_COMMAND:+; $PROMPT_COMMAND}" ;;
esac
"#;
