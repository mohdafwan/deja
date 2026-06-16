//! Shell integration scripts. `deja init <shell>` inhe print karta hai;
//! user apne rc file me `eval "$(deja init bash)"` lagata hai.
//!
//! Phase 0.1: sirf command + exit code capture (har prompt pe `deja record` call).
//! Duration/snapshot baad ke phases me. Hook background me (`&`) chalta hai taaki
//! prompt slow na ho.

use anyhow::{bail, Result};

pub fn init_script(shell: &str) -> Result<&'static str> {
    match shell {
        "bash" => Ok(BASH),
        "zsh" => Ok(ZSH),
        other => bail!("'{other}' shell abhi support nahi (try: bash | zsh)"),
    }
}

const BASH: &str = r#"# >>> deja shell integration (bash) >>>
# duration: PS0 har command se theek pehle start-time likhta hai (bash 5+).
__DEJA_T0F="${TMPDIR:-/tmp}/.deja_t0_$$"
__deja_mark() { printf '%s' "${EPOCHREALTIME:-}" > "$__DEJA_T0F" 2>/dev/null; }
case "$PS0" in
  *__deja_mark*) ;;
  *) PS0='$(__deja_mark)'"$PS0" ;;
esac
__deja_precmd() {
  local __deja_ec=$?
  local __deja_hnum __deja_cmd
  read -r __deja_hnum __deja_cmd <<< "$(HISTTIMEFORMAT='' history 1)"
  if [ -n "$__deja_cmd" ] && [ "$__deja_hnum" != "$__DEJA_LAST_HNUM" ]; then
    __DEJA_LAST_HNUM="$__deja_hnum"
    local __deja_dur=-1
    if [ -r "$__DEJA_T0F" ]; then
      local __deja_t0; __deja_t0="$(cat "$__DEJA_T0F" 2>/dev/null)"; rm -f "$__DEJA_T0F"
      if [ -n "$__deja_t0" ] && [ -n "${EPOCHREALTIME:-}" ]; then
        local __deja_t1="${EPOCHREALTIME//,/.}"; __deja_t0="${__deja_t0//,/.}"
        __deja_dur="$(LC_ALL=C awk "BEGIN{d=($__deja_t1-$__deja_t0)*1000; if(d<0)d=0; printf \"%d\", d}")"
      fi
    fi
    if [ "$__deja_ec" -eq 0 ]; then
      # success: chup-chaap background me record
      deja record --command "$__deja_cmd" --exit "$__deja_ec" --cwd "$PWD" \
        --started-at "$(date +%s)" --duration-ms "$__deja_dur" >/dev/null 2>&1 &
    else
      # failure: foreground (taaki diff dikhe) + --explain
      deja record --command "$__deja_cmd" --exit "$__deja_ec" --cwd "$PWD" \
        --started-at "$(date +%s)" --duration-ms "$__deja_dur" --explain
    fi
  fi
}
case ";${PROMPT_COMMAND};" in
  *";__deja_precmd;"*) ;;
  *) PROMPT_COMMAND="__deja_precmd${PROMPT_COMMAND:+; $PROMPT_COMMAND}" ;;
esac
# <<< deja shell integration (bash) <<<
"#;

const ZSH: &str = r#"# >>> deja shell integration (zsh) >>>
zmodload zsh/datetime 2>/dev/null
__deja_preexec() { __DEJA_CMD="$1"; __DEJA_T0="${EPOCHREALTIME:-}"; }
__deja_precmd() {
  local __deja_ec=$?
  if [ -n "$__DEJA_CMD" ]; then
    local __deja_dur=-1
    if [ -n "$__DEJA_T0" ] && [ -n "${EPOCHREALTIME:-}" ]; then
      __deja_dur=$(( int((EPOCHREALTIME - __DEJA_T0) * 1000) ))
    fi
    if [ "$__deja_ec" -eq 0 ]; then
      deja record --command "$__DEJA_CMD" --exit "$__deja_ec" --cwd "$PWD" \
        --started-at "$(date +%s)" --duration-ms "$__deja_dur" >/dev/null 2>&1 &!
    else
      deja record --command "$__DEJA_CMD" --exit "$__deja_ec" --cwd "$PWD" \
        --started-at "$(date +%s)" --duration-ms "$__deja_dur" --explain
    fi
    __DEJA_CMD=""
  fi
}
autoload -Uz add-zsh-hook
add-zsh-hook preexec __deja_preexec
add-zsh-hook precmd __deja_precmd
# <<< deja shell integration (zsh) <<<
"#;
