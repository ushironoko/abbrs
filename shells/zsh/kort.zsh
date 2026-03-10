# kort - Fast and safe abbreviation expansion for zsh
# Source this file in your .zshrc:
#   source /path/to/kort.zsh

# Expand abbreviation on Space key
kort-expand-space() {
  local -a out
  out=( "${(f)$(kort expand --lbuffer="$LBUFFER" --rbuffer="$RBUFFER")}" )

  if [[ $out[1] == stale_cache ]]; then
    kort compile 2>/dev/null
    out=( "${(f)$(kort expand --lbuffer="$LBUFFER" --rbuffer="$RBUFFER")}" )
  fi

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

  if [[ $out[1] == stale_cache ]]; then
    kort compile 2>/dev/null
    out=( "${(f)$(kort expand --lbuffer="$LBUFFER" --rbuffer="$RBUFFER")}" )
  fi

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
    'expand:Expand abbreviation (called from ZLE)'
    'next-placeholder:Jump to next placeholder'
    'list:List registered abbreviations'
    'check:Syntax check config only'
    'init:Initialize kort'
    'add:Add a new abbreviation'
    'erase:Erase an abbreviation'
    'rename:Rename an abbreviation'
    'query:Query if abbreviation exists'
    'show:Show abbreviations'
    'remind:Check for abbreviation reminders'
    'import:Import abbreviations'
    'export:Export abbreviations'
  )

  _kort_keywords() {
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
    keywords=( ${(f)"$(kort _list-keywords "${cfg_flag[@]}" 2>/dev/null)"} )
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
        '1:keyword:_kort_keywords' && return
      ;;
    rename)
      _arguments -s \
        '--command=[Only rename command-scoped entry]:command:' \
        '--global[Only rename global entry]' \
        '--config=[Config file path]:config file:_files' \
        '1:old keyword:_kort_keywords' \
        '2:new keyword:' && return
      ;;
    query)
      _arguments -s \
        '--command=[Only query command-scoped entry]:command:' \
        '--global[Only query global entry]' \
        '--config=[Config file path]:config file:_files' \
        '1:keyword:_kort_keywords' && return
      ;;
    show)
      _arguments -s \
        '--config=[Config file path]:config file:_files' \
        '1:keyword:_kort_keywords' && return
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
(( $+functions[compdef] )) && compdef _kort kort
