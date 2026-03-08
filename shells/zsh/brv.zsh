# brv - Fast and safe abbreviation expansion for zsh
# Source this file in your .zshrc:
#   source /path/to/brv.zsh

# Expand abbreviation on Space key
brv-expand-space() {
  local -a out
  out=( "${(f)$(brv expand --lbuffer="$LBUFFER" --rbuffer="$RBUFFER")}" )

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
    stale_cache)
      # Recompile if cache is stale
      brv compile 2>/dev/null
      # Retry
      out=( "${(f)$(brv expand --lbuffer="$LBUFFER" --rbuffer="$RBUFFER")}" )
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
brv-expand-accept() {
  local -a out
  out=( "${(f)$(brv expand --lbuffer="$LBUFFER" --rbuffer="$RBUFFER")}" )

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
    stale_cache)
      brv compile 2>/dev/null
      out=( "${(f)$(brv expand --lbuffer="$LBUFFER" --rbuffer="$RBUFFER")}" )
      if [[ $out[1] == "success" && -n $out[2] ]]; then
        BUFFER=$out[2]
      fi
      ;;
  esac

  zle accept-line
}

# Jump to next placeholder on Tab key
brv-next-placeholder() {
  local -a out
  out=( "${(f)$(brv next-placeholder --lbuffer="$LBUFFER" --rbuffer="$RBUFFER")}" )
  if [[ $out[1] == "success" && -n $out[2] ]]; then
    BUFFER=$out[2]
    CURSOR=$out[3]
  else
    # Fall back to normal tab completion if no placeholder
    zle expand-or-complete
  fi
}

# Register widgets
zle -N brv-expand-space
zle -N brv-expand-accept
zle -N brv-next-placeholder

# Key bindings
bindkey " " brv-expand-space
bindkey "^M" brv-expand-accept
bindkey "^I" brv-next-placeholder
