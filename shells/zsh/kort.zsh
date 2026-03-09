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
