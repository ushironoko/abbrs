# kort - Fast and safe abbreviation expansion for zsh
# Source this file in your .zshrc:
#   source /path/to/kort.zsh

# Expand abbreviation on Space key
kort-expand-space() {
  local -a out
  out=( "${(f)$(kort expand --lbuffer="$LBUFFER" --rbuffer="$RBUFFER")}" )

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
      # Command evaluation
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
      # Shell function call
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
    stale_cache)
      # Recompile if cache is stale
      kort compile 2>/dev/null
      # Retry
      out=( "${(f)$(kort expand --lbuffer="$LBUFFER" --rbuffer="$RBUFFER")}" )
      if [[ $out[1] == "success" && -n $out[2] ]]; then
        BUFFER=$out[2]
        CURSOR=$out[3]
      else
        zle self-insert
      fi
      ;;
    candidates)
      local count=$out[2]
      local msg=""
      local i
      for (( i=3; i <= count + 2; i++ )); do
        local kw="${out[$i]%%	*}"
        local exp="${out[$i]#*	}"
        msg+="  ${kw} → ${exp}"$'\n'
      done
      zle -M "$msg"
      # Do not insert space — user continues typing to narrow down
      ;;
    *)
      zle self-insert
      ;;
  esac
}

# Expand abbreviation on Enter key and execute
kort-expand-accept() {
  local -a out
  out=( "${(f)$(kort expand --lbuffer="$LBUFFER" --rbuffer="$RBUFFER")}" )

  case $out[1] in
    success)
      if [[ -n $out[2] ]]; then
        BUFFER=$out[2]
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
    stale_cache)
      kort compile 2>/dev/null
      out=( "${(f)$(kort expand --lbuffer="$LBUFFER" --rbuffer="$RBUFFER")}" )
      if [[ $out[1] == "success" && -n $out[2] ]]; then
        BUFFER=$out[2]
      fi
      ;;
  esac

  # Check for reminders before accepting
  local remind_msg
  remind_msg=$(kort remind --buffer="$BUFFER" 2>/dev/null)
  if [[ -n $remind_msg ]]; then
    zle -M "$remind_msg"
  fi

  zle accept-line
}

# Jump to next placeholder on Tab key
kort-next-placeholder() {
  local -a out
  out=( "${(f)$(kort next-placeholder --lbuffer="$LBUFFER" --rbuffer="$RBUFFER")}" )
  if [[ $out[1] == "success" && -n $out[2] ]]; then
    BUFFER=$out[2]
    CURSOR=$out[3]
  else
    # Fall back to normal tab completion if no placeholder
    zle expand-or-complete
  fi
}

# Literal space (no expansion)
kort-literal-space() {
  zle self-insert
}

# Register widgets
zle -N kort-expand-space
zle -N kort-expand-accept
zle -N kort-next-placeholder
zle -N kort-literal-space

# Key bindings
bindkey " " kort-expand-space
bindkey "^M" kort-expand-accept
bindkey "^I" kort-next-placeholder
bindkey "^ " kort-literal-space

# Zsh completion function
_kort() {
  local -a subcmds
  subcmds=(
    'compile:Compile config and verify conflicts'
    'list:List registered abbreviations'
    'check:Syntax check config only'
    'init:Initialize kort'
    'add:Add a new abbreviation'
    'erase:Erase an abbreviation'
    'rename:Rename an abbreviation'
    'query:Query if abbreviation exists'
    'show:Show abbreviations'
    'import:Import abbreviations'
    'export:Export abbreviations'
  )
  if (( CURRENT == 2 )); then
    _describe 'subcommand' subcmds
    return
  fi
  case $words[2] in
    erase|show|query)
      if (( CURRENT == 3 )); then
        local -a keywords
        keywords=( ${(f)"$(kort _list-keywords 2>/dev/null)"} )
        _describe 'keyword' keywords
      fi
      ;;
    rename)
      if (( CURRENT == 3 )); then
        local -a keywords
        keywords=( ${(f)"$(kort _list-keywords 2>/dev/null)"} )
        _describe 'keyword' keywords
      fi
      ;;
    init)
      if (( CURRENT == 3 )); then
        local -a targets=('zsh:Output zsh integration script' 'config:Generate config template')
        _describe 'target' targets
      fi
      ;;
    import)
      if (( CURRENT == 3 )); then
        local -a sources=('aliases:Import from zsh aliases' 'fish:Import from fish' 'git-aliases:Import from git aliases')
        _describe 'source' sources
      fi
      ;;
  esac
}
(( $+functions[compdef] )) && compdef _kort kort
