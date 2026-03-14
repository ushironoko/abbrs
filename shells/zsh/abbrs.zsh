# abbrs - Fast and safe abbreviation expansion for zsh
# Source this file in your .zshrc:
#   source /path/to/abbrs.zsh

# --- Binary path (replaced by `abbrs init zsh`) ---

typeset -g _ABBRS_BIN="__ABBRS_BIN__"
# Fallback: if placeholder was not replaced (e.g. sourced directly), find abbrs in PATH
if [[ $_ABBRS_BIN == "__ABBRS_BIN__" ]]; then
  _ABBRS_BIN="${commands[abbrs]:-abbrs}"
fi

# --- Socket-based serve management ---
# Uses Unix domain socket + &! (disown) to avoid polluting the job table.
# This fixes `wait` (no args) hanging when coproc was used.

typeset -g _ABBRS_SOCK_DIR="${TMPDIR:-/tmp}/abbrs-$(id -u)"
typeset -g _ABBRS_SOCK="${_ABBRS_SOCK_DIR}/abbrs-$$.sock"
typeset -g _ABBRS_SERVE_PID=0
typeset -g _ABBRS_SOCK_FD=""

# --- Candidate cycling state ---

typeset -g  _ABBRS_CYCLING=0
typeset -ga _ABBRS_CANDIDATES=()
typeset -g  _ABBRS_CYCLE_INDEX=0
typeset -g  _ABBRS_CYCLE_LPREFIX=""
typeset -g  _ABBRS_CYCLE_RBUFFER=""
typeset -g  _ABBRS_CYCLE_ORIG_TOKEN=""

_abbrs_start_serve() {
  # Don't start daemon if zsocket is unavailable
  zmodload zsh/net/socket 2>/dev/null || return 1

  _abbrs_stop_serve
  $_ABBRS_BIN serve --socket "$_ABBRS_SOCK" &!
  _ABBRS_SERVE_PID=$!
  # Wait for socket file to become available (max ~100ms)
  local i
  for (( i=0; i<50; i++ )); do
    [[ -S "$_ABBRS_SOCK" ]] && break
    command sleep 0.002
  done
  if [[ -S "$_ABBRS_SOCK" ]]; then
    _abbrs_connect || {
      # Connection failed — kill the daemon we just spawned
      kill $_ABBRS_SERVE_PID 2>/dev/null
      _ABBRS_SERVE_PID=0
      command rm -f "$_ABBRS_SOCK"
      return 1
    }
  else
    # Startup timed out — kill the spawned daemon before giving up
    kill $_ABBRS_SERVE_PID 2>/dev/null
    _ABBRS_SERVE_PID=0
    command rm -f "$_ABBRS_SOCK"
    return 1
  fi
}

_abbrs_connect() {
  zmodload zsh/net/socket 2>/dev/null || return 1
  zsocket "$_ABBRS_SOCK" 2>/dev/null || return 1
  _ABBRS_SOCK_FD=$REPLY
}

_abbrs_stop_serve() {
  if [[ -n "$_ABBRS_SOCK_FD" ]]; then
    exec {_ABBRS_SOCK_FD}>&-
    _ABBRS_SOCK_FD=""
  fi
  if (( _ABBRS_SERVE_PID > 0 )); then
    kill $_ABBRS_SERVE_PID 2>/dev/null
    _ABBRS_SERVE_PID=0
  fi
  command rm -f "$_ABBRS_SOCK"
}

if (( $+functions[add-zsh-hook] )); then
  add-zsh-hook zshexit _abbrs_stop_serve
else
  zshexit() { _abbrs_stop_serve }
fi

# --- Socket communication ---

typeset -ga _abbrs_reply

_abbrs_request() {
  local request="$1"
  _abbrs_reply=()

  # Check if serve process is alive; restart if needed
  if (( _ABBRS_SERVE_PID <= 0 )) || ! kill -0 $_ABBRS_SERVE_PID 2>/dev/null; then
    _abbrs_start_serve || return 1
  fi

  # Reconnect if socket fd is closed
  if [[ -z "$_ABBRS_SOCK_FD" ]]; then
    _abbrs_connect || return 1
  fi

  # Send request
  print -ru $_ABBRS_SOCK_FD "$request" 2>/dev/null || {
    # Connection broken — try reconnect once
    _ABBRS_SOCK_FD=""
    _abbrs_connect || return 1
    print -ru $_ABBRS_SOCK_FD "$request" 2>/dev/null || return 1
  }

  # Read response lines until EOR (\x1e)
  local line
  while true; do
    read -ru $_ABBRS_SOCK_FD -t 1 line 2>/dev/null || return 1
    if [[ $line == $'\x1e'* ]]; then
      break
    fi
    _abbrs_reply+=( "$line" )
  done
  return 0
}

# --- Fallback (per-process mode) ---

_abbrs_expand_fallback() {
  local -a out
  out=( "${(f)$($_ABBRS_BIN expand --lbuffer="$LBUFFER" --rbuffer="$RBUFFER")}" )

  if [[ $out[1] == stale_cache ]]; then
    $_ABBRS_BIN compile 2>/dev/null
    out=( "${(f)$($_ABBRS_BIN expand --lbuffer="$LBUFFER" --rbuffer="$RBUFFER")}" )
  fi

  echo "${(F)out}"
}

_abbrs_placeholder_fallback() {
  $_ABBRS_BIN next-placeholder --lbuffer="$LBUFFER" --rbuffer="$RBUFFER"
}

_abbrs_remind_fallback() {
  $_ABBRS_BIN remind --buffer="$1" 2>/dev/null
}

# --- Candidate cycling helpers ---

_abbrs_clear_candidates() {
  if (( _ABBRS_CYCLING )); then
    local restore=${1:-0}
    if (( restore )); then
      # Cancel: restore original token
      LBUFFER="${_ABBRS_CYCLE_LPREFIX}${_ABBRS_CYCLE_ORIG_TOKEN}"
      RBUFFER="$_ABBRS_CYCLE_RBUFFER"
    fi
    _ABBRS_CYCLING=0
    _ABBRS_CANDIDATES=()
    _ABBRS_CYCLE_INDEX=0
    _ABBRS_CYCLE_LPREFIX=""
    _ABBRS_CYCLE_RBUFFER=""
    _ABBRS_CYCLE_ORIG_TOKEN=""
    zle -M ""
  fi
}

_abbrs_cycle_next() {
  (( _ABBRS_CYCLE_INDEX = (_ABBRS_CYCLE_INDEX % $#_ABBRS_CANDIDATES) + 1 ))
  _abbrs_apply_cycle
}

_abbrs_cycle_prev() {
  (( _ABBRS_CYCLE_INDEX = (_ABBRS_CYCLE_INDEX - 2 + $#_ABBRS_CANDIDATES) % $#_ABBRS_CANDIDATES + 1 ))
  _abbrs_apply_cycle
}

_abbrs_apply_cycle() {
  local selected="${_ABBRS_CANDIDATES[$_ABBRS_CYCLE_INDEX]}"
  local kw="${selected%%	*}"

  LBUFFER="${_ABBRS_CYCLE_LPREFIX}${kw}"
  RBUFFER="$_ABBRS_CYCLE_RBUFFER"

  local msg="" i
  for (( i=1; i <= $#_ABBRS_CANDIDATES; i++ )); do
    local ckw="${_ABBRS_CANDIDATES[$i]%%	*}"
    local cexp="${_ABBRS_CANDIDATES[$i]#*	}"
    if (( i > 1 )); then
      msg+=$'\n'
    fi
    if (( i == _ABBRS_CYCLE_INDEX )); then
      msg+="▸ ${ckw} → ${cexp}"
    else
      msg+="  ${ckw} → ${cexp}"
    fi
  done
  zle -M "$msg"
}

# --- Response handling ---

_abbrs_handle_expand_response() {
  local -a out
  out=( "$@" )

  case $out[1] in
    success)
      if [[ -n $out[2] ]]; then
        BUFFER=$out[2]
        CURSOR=$out[3]
      else
        zle self-insert
      fi
      ;;
    evaluate)
      local result
      result=$(eval "$out[2]" 2>/dev/null)
      if [[ -n $result ]]; then
        BUFFER="${out[3]}${result}${out[4]}"
        CURSOR=$(( ${#out[3]} + ${#result} ))
      else
        zle self-insert
      fi
      ;;
    function)
      if ! whence -w "$out[2]" >/dev/null 2>&1; then
        zle self-insert
        return
      fi
      local result
      result=$("$out[2]" "$out[3]" 2>/dev/null)
      if [[ -n $result ]]; then
        BUFFER="${out[4]}${result}${out[5]}"
        CURSOR=$(( ${#out[4]} + ${#result} ))
      else
        zle self-insert
      fi
      ;;
    candidates)
      local count=$out[2]
      _ABBRS_CANDIDATES=()
      local i
      for (( i=3; i <= count + 2; i++ )); do
        _ABBRS_CANDIDATES+=( "$out[$i]" )
      done

      local lbuf="$LBUFFER"
      if [[ "$lbuf" == *" "* ]]; then
        _ABBRS_CYCLE_LPREFIX="${lbuf% *} "
        _ABBRS_CYCLE_ORIG_TOKEN="${lbuf##* }"
      else
        _ABBRS_CYCLE_LPREFIX=""
        _ABBRS_CYCLE_ORIG_TOKEN="$lbuf"
      fi
      _ABBRS_CYCLE_RBUFFER="$RBUFFER"
      _ABBRS_CYCLING=1
      _ABBRS_CYCLE_INDEX=0

      local msg=""
      for (( i=1; i <= $#_ABBRS_CANDIDATES; i++ )); do
        local kw="${_ABBRS_CANDIDATES[$i]%%	*}"
        local exp="${_ABBRS_CANDIDATES[$i]#*	}"
        if (( i > 1 )); then
          msg+=$'\n'
        fi
        msg+="  ${kw} → ${exp}"
      done
      zle -M "$msg"
      ;;
    *)
      zle self-insert
      ;;
  esac
}

_abbrs_handle_expand_accept_response() {
  local -a out
  out=( "$@" )

  case $out[1] in
    success)
      if [[ -n $out[2] ]]; then
        BUFFER=$out[2]
        CURSOR=$out[3]
      fi
      ;;
    evaluate)
      local result
      result=$(eval "$out[2]" 2>/dev/null)
      if [[ -n $result ]]; then
        BUFFER="${out[3]}${result}${out[4]}"
      fi
      ;;
    function)
      if whence -w "$out[2]" >/dev/null 2>&1; then
        local result
        result=$("$out[2]" "$out[3]" 2>/dev/null)
        if [[ -n $result ]]; then
          BUFFER="${out[4]}${result}${out[5]}"
        fi
      fi
      ;;
  esac
}

# --- Widget functions ---

# Expand with stale_cache retry logic. Takes a response handler function name.
_abbrs_expand_with_fallback() {
  local handler="$1"

  if _abbrs_request $'expand\t'"${LBUFFER}"$'\t'"${RBUFFER}"; then
    if [[ ${_abbrs_reply[1]} == stale_cache ]]; then
      $_ABBRS_BIN compile 2>/dev/null
      _abbrs_request "reload"
      if _abbrs_request $'expand\t'"${LBUFFER}"$'\t'"${RBUFFER}"; then
        "$handler" "${_abbrs_reply[@]}"
      else
        local -a fb
        fb=( "${(f)$(_abbrs_expand_fallback)}" )
        "$handler" "${fb[@]}"
      fi
    else
      "$handler" "${_abbrs_reply[@]}"
    fi
  else
    local -a fb
    fb=( "${(f)$(_abbrs_expand_fallback)}" )
    "$handler" "${fb[@]}"
  fi
}

# Expand abbreviation on Space key
abbrs-expand-space() {
  if (( _ABBRS_CYCLING )); then
    _abbrs_clear_candidates
  fi
  _abbrs_expand_with_fallback _abbrs_handle_expand_response
}

# Expand abbreviation on Enter key and execute
abbrs-expand-accept() {
  if (( _ABBRS_CYCLING )); then
    _abbrs_clear_candidates
    _abbrs_expand_with_fallback _abbrs_handle_expand_accept_response
    return
  fi
  _abbrs_expand_with_fallback _abbrs_handle_expand_accept_response

  # Check for reminders before accepting
  if _abbrs_request $'remind\t'"${BUFFER}"; then
    if [[ -n ${_abbrs_reply[1]} ]]; then
      zle -M "${_abbrs_reply[1]}"
    fi
  else
    local remind_msg
    remind_msg=$(_abbrs_remind_fallback "$BUFFER")
    if [[ -n $remind_msg ]]; then
      zle -M "$remind_msg"
    fi
  fi

  if [[ "$BUFFER" == exit || "$BUFFER" == logout ]]; then
    _abbrs_stop_serve
  fi

  zle accept-line
}

# Jump to next placeholder on Tab key
abbrs-next-placeholder() {
  # Priority 1: Candidate cycling (skip placeholder check during cycling)
  if (( _ABBRS_CYCLING )); then
    _abbrs_cycle_next
    return
  fi

  # Priority 2: Placeholder jump
  if _abbrs_request $'placeholder\t'"${LBUFFER}"$'\t'"${RBUFFER}"; then
    if [[ ${_abbrs_reply[1]} == "success" && -n ${_abbrs_reply[2]} ]]; then
      BUFFER=${_abbrs_reply[2]}
      CURSOR=${_abbrs_reply[3]}
      return
    fi
  else
    local -a out
    out=( "${(f)$(_abbrs_placeholder_fallback)}" )
    if [[ $out[1] == "success" && -n $out[2] ]]; then
      BUFFER=$out[2]
      CURSOR=$out[3]
      return
    fi
  fi

  # Priority 3: Shell completion
  zle expand-or-complete
}

# Reverse cycle candidates on Shift+Tab
abbrs-prev-candidate() {
  if (( _ABBRS_CYCLING )); then
    _abbrs_cycle_prev
  fi
}

# Show expansion history and enter candidate cycling
abbrs-history() {
  if (( _ABBRS_CYCLING )); then
    _abbrs_clear_candidates 1
  fi

  if _abbrs_request "history"; then
    if [[ ${_abbrs_reply[1]} == "candidates" ]]; then
      _abbrs_handle_expand_response "${_abbrs_reply[@]}"
    else
      zle -M "abbrs: no expansion history"
    fi
  else
    zle -M "abbrs: history requires serve mode"
  fi
}

# Literal space (no expansion)
abbrs-literal-space() {
  _abbrs_clear_candidates 1
  zle self-insert
}

# Register widgets
zle -N abbrs-expand-space
zle -N abbrs-expand-accept
zle -N abbrs-next-placeholder
zle -N abbrs-prev-candidate
zle -N abbrs-literal-space
zle -N abbrs-history

# Key bindings
bindkey " " abbrs-expand-space
bindkey "^M" abbrs-expand-accept
bindkey "^I" abbrs-next-placeholder
bindkey "^[[Z" abbrs-prev-candidate
bindkey "^ " abbrs-literal-space
bindkey "^X^H" abbrs-history

# Cancel candidate cycling on any non-abbrs keypress
_abbrs_check_cycling() {
  if (( _ABBRS_CYCLING )); then
    case "$LASTWIDGET" in
      abbrs-expand-space|abbrs-expand-accept|abbrs-next-placeholder|abbrs-prev-candidate|abbrs-literal-space|abbrs-history)
        ;;
      *)
        # Accept current candidate (don't restore) so the user's keystroke is preserved.
        # Only explicit cancel (abbrs-literal-space / Ctrl+Space) restores the original token.
        _abbrs_clear_candidates 0
        ;;
    esac
  fi
}
zle -N _abbrs_check_cycling
# Try to load add-zle-hook-widget (shipped with zsh >=5.3) for proper hook chaining.
# +X forces immediate loading so $+functions is only true when the file actually exists in fpath.
autoload -Uz +X add-zle-hook-widget 2>/dev/null
if (( $+functions[add-zle-hook-widget] )); then
  add-zle-hook-widget line-pre-redraw _abbrs_check_cycling
else
  # Fallback for ancient zsh without add-zle-hook-widget
  zle -N zle-line-pre-redraw _abbrs_check_cycling
fi

# Start serve process on load (socket mode if zsocket available and serve enabled)
if zmodload zsh/net/socket 2>/dev/null && $_ABBRS_BIN _serve-enabled 2>/dev/null; then
  _abbrs_start_serve
else
  # zsocket not available or serve disabled — per-process fallback only (no background daemon)
  # _abbrs_request will always fail, so widgets fall through to _abbrs_*_fallback
  :
fi

# Zsh completion function
_abbrs() {
  local -a subcmds
  subcmds=(
    'compile:Compile config and verify conflicts'
    'expand:Expand abbreviation (called from ZLE)'
    'next-placeholder:Jump to next placeholder'
    'list:List registered abbreviations'
    'check:Syntax check config only'
    'init:Initialize abbrs'
    'add:Add a new abbreviation'
    'erase:Erase an abbreviation'
    'rename:Rename an abbreviation'
    'query:Query if abbreviation exists'
    'show:Show abbreviations'
    'remind:Check for abbreviation reminders'
    'import:Import abbreviations'
    'export:Export abbreviations'
    'history:Manage expansion history'
    'serve:Start serve mode'
  )

  _abbrs_keywords() {
    local -a cfg_flag keywords
    local i config_val
    cfg_flag=()
    for (( i=2; i < $#words; i++ )); do
      if [[ $words[$i] == --config && -n $words[$((i+1))] ]]; then
        config_val=$words[$((i+1))]
        cfg_flag=( --config "$config_val" )
        break
      elif [[ $words[$i] == --config=* ]]; then
        config_val=${words[$i]#--config=}
        cfg_flag=( --config "$config_val" )
        break
      fi
    done
    keywords=( ${(f)"$($_ABBRS_BIN _list-keywords "${cfg_flag[@]}" 2>/dev/null)"} )
    _describe 'keyword' keywords
  }

  if (( CURRENT == 2 )); then
    _describe 'subcommand' subcmds
    return
  fi

  case $words[2] in
    compile|list|check|export)
      _arguments -s \
        '--config=[Config file path]:config file:_files' \
        '*:' && return
      ;;
    expand)
      _arguments -s \
        '--lbuffer=[Buffer left of cursor]:lbuffer:' \
        '--rbuffer=[Buffer right of cursor]:rbuffer:' \
        '--cache=[Cache file path]:cache file:_files' \
        '--config=[Config file path]:config file:_files' \
        '*:' && return
      ;;
    next-placeholder)
      _arguments -s \
        '--lbuffer=[Buffer left of cursor]:lbuffer:' \
        '--rbuffer=[Buffer right of cursor]:rbuffer:' \
        '*:' && return
      ;;
    serve)
      _arguments -s \
        '--socket=[Unix domain socket path]:socket file:_files' \
        '--cache=[Cache file path]:cache file:_files' \
        '--config=[Config file path]:config file:_files' \
        '*:' && return
      ;;
    add)
      _arguments -s \
        '--global[Register as global abbreviation]' \
        '--evaluate[Run expansion as command]' \
        '--function[Run expansion as shell function]' \
        '--regex[Keyword is a regex pattern]' \
        '--command=[Only expand as argument of this command]:command:' \
        '--allow-conflict[Allow conflict with PATH commands]' \
        '--context-lbuffer=[Context lbuffer regex]:pattern:' \
        '--context-rbuffer=[Context rbuffer regex]:pattern:' \
        '--config=[Config file path]:config file:_files' \
        '1:keyword:' \
        '2:expansion:' && return
      ;;
    erase)
      _arguments -s \
        '--command=[Only erase command-scoped entry]:command:' \
        '--global[Only erase global entry]' \
        '--config=[Config file path]:config file:_files' \
        '1:keyword:_abbrs_keywords' && return
      ;;
    rename)
      _arguments -s \
        '--command=[Only rename command-scoped entry]:command:' \
        '--global[Only rename global entry]' \
        '--config=[Config file path]:config file:_files' \
        '1:old keyword:_abbrs_keywords' \
        '2:new keyword:' && return
      ;;
    query)
      _arguments -s \
        '--command=[Only query command-scoped entry]:command:' \
        '--global[Only query global entry]' \
        '--config=[Config file path]:config file:_files' \
        '1:keyword:_abbrs_keywords' && return
      ;;
    show)
      _arguments -s \
        '--config=[Config file path]:config file:_files' \
        '1:keyword:_abbrs_keywords' && return
      ;;
    remind)
      _arguments -s \
        '--buffer=[Full buffer]:buffer:' \
        '--cache=[Cache file path]:cache file:_files' \
        '*:' && return
      ;;
    init)
      if (( CURRENT == 3 )); then
        local -a targets=('zsh:Output zsh integration script' 'config:Generate config template')
        _describe 'target' targets
      fi
      ;;
    history)
      if (( CURRENT == 3 )); then
        local -a actions=('list:List recent expansion history' 'clear:Clear all expansion history')
        _describe 'action' actions
        return
      fi
      case $words[3] in
        list)
          _arguments -s \
            '-n=[Maximum entries]:limit:' \
            '--limit=[Maximum entries]:limit:' \
            '--config=[Config file path]:config file:_files' \
            '*:' && return
          ;;
      esac
      ;;
    import)
      if (( CURRENT == 3 )); then
        local -a sources=('aliases:Import from zsh aliases' 'fish:Import from fish' 'git-aliases:Import from git aliases')
        _describe 'source' sources
        return
      fi
      # Shift words so _arguments sees the sub-subcommand as the command
      (( CURRENT -= 2 ))
      shift 2 words
      case $words[1] in
        fish)
          _arguments -s \
            '--config=[Config file path]:config file:_files' \
            ':input file:_files' && return
          ;;
        aliases|git-aliases)
          _arguments -s \
            '--config=[Config file path]:config file:_files' && return
          ;;
      esac
      ;;
  esac
}
(( $+functions[compdef] )) && compdef _abbrs abbrs
