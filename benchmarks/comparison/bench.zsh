#!/usr/bin/env zsh
# =============================================================================
# kort vs zsh-abbr comparison benchmark
# =============================================================================
# Measures end-to-end expansion latency as experienced by the user:
#   - kort:     fork+exec `kort expand` → cache read → HashMap lookup → stdout
#   - zsh-abbr: in-process function call → associative array lookup
#   - raw zsh:  direct associative array access (theoretical lower bound)
# =============================================================================

zmodload zsh/datetime  # provides $EPOCHREALTIME (microsecond precision)
set -u

# ---------------------------------------------------------------------------
# Config
# ---------------------------------------------------------------------------
SCRIPT_DIR=${0:a:h}
PROJECT_ROOT=${SCRIPT_DIR:h:h}
KORT_BIN=${PROJECT_ROOT}/target/release/kort
BENCH_TMPDIR=$(mktemp -d)
ITERATIONS=${1:-1000}
SIZES=(10 50 100 500)

trap 'rm -rf $BENCH_TMPDIR' EXIT

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------
print_header() {
  printf '\n\033[1;36m%s\033[0m\n' "$1"
  printf '%.0s─' {1..70}
  printf '\n'
}

print_row() {
  # $1=label  $2=total_sec  $3=iterations
  local avg_us=$(( $2 / $3 * 1000000 ))
  local per_op_ms=$(( $2 / $3 * 1000 ))
  printf '  %-35s %10.1f µs/op  (%8.3f ms/op)\n' "$1" "$avg_us" "$per_op_ms"
}

print_row_compare() {
  # $1=label  $2=total_sec  $3=iterations  $4=baseline_total_sec
  local avg_us=$(( $2 / $3 * 1000000 ))
  local per_op_ms=$(( $2 / $3 * 1000 ))
  local ratio
  if (( $4 > 0 )); then
    ratio=$(( $2 / $4 ))
    printf '  %-35s %10.1f µs/op  (%8.3f ms/op)  %.2fx\n' "$1" "$avg_us" "$per_op_ms" "$ratio"
  else
    printf '  %-35s %10.1f µs/op  (%8.3f ms/op)\n' "$1" "$avg_us" "$per_op_ms"
  fi
}

# ---------------------------------------------------------------------------
# Build kort
# ---------------------------------------------------------------------------
if [[ ! -x $KORT_BIN ]]; then
  echo "Building kort (release)..."
  (cd $PROJECT_ROOT && cargo build --release 2>&1)
fi
echo "kort binary: $KORT_BIN"
echo "Iterations per measurement: $ITERATIONS"

# ---------------------------------------------------------------------------
# Load zsh-abbr (non-interactive, bindings disabled)
# ---------------------------------------------------------------------------
ABBR_DEFAULT_BINDINGS=0
ABBR_AUTOLOAD=0
ABBR_GET_AVAILABLE_ABBREVIATION=0
source /opt/homebrew/share/zsh-abbr/zsh-abbr.zsh 2>/dev/null || true

# ---------------------------------------------------------------------------
# Warmup kort binary (page cache)
# ---------------------------------------------------------------------------
warmup_kort() {
  local cache_path=$1
  local config_path=$2
  for _w in {1..5}; do
    $KORT_BIN expand --lbuffer "warmup" --rbuffer "" --cache "$cache_path" --config "$config_path" >/dev/null 2>&1 || true
  done
}

# =============================================================================
# Main benchmark loop
# =============================================================================
printf '\n'
printf '╔══════════════════════════════════════════════════════════════════════╗\n'
printf '║           kort vs zsh-abbr  Expansion Benchmark                    ║\n'
printf '╚══════════════════════════════════════════════════════════════════════╝\n'

# Collect results for summary table
typeset -A results_kort results_kort_serve results_abbr results_raw

# Pre-declare loop variables to avoid zsh typeset output on re-declaration
local bench_start bench_end
local target_keyword kort_dir kort_config kort_cache
local kort_total kort_serve_total abbr_total raw_total quoted_target
local serve_in serve_out serve_pid serve_fd_w serve_fd_r _sline

for SIZE in $SIZES; do
  print_header "Abbreviation count: $SIZE"

  target_keyword="abbr$((SIZE / 2))"

  # =========================================================================
  # Setup: kort
  # =========================================================================
  kort_dir="${BENCH_TMPDIR}/kort_${SIZE}"
  mkdir -p "${kort_dir}/config" "${kort_dir}/cache/kort"
  kort_config="${kort_dir}/config/kort.toml"
  kort_cache="${kort_dir}/cache/kort/kort.cache"

  # Generate kort.toml
  {
    echo '[settings]'
    echo ''
    for i in $(seq 0 $((SIZE - 1))); do
      echo "[[abbr]]"
      echo "keyword = \"abbr${i}\""
      echo "expansion = \"expanded command ${i} with some arguments\""
      echo ""
    done
  } > "$kort_config"

  # Compile (set XDG paths so cache goes where we want)
  XDG_CONFIG_HOME="${kort_dir}/config" XDG_CACHE_HOME="${kort_dir}/cache" \
    $KORT_BIN compile --config "$kort_config" 2>/dev/null

  # Verify cache exists
  if [[ ! -f "$kort_cache" ]]; then
    echo "ERROR: kort cache not created at $kort_cache"
    ls -la "${kort_dir}/cache/" 2>&1
    continue
  fi

  # Warmup
  warmup_kort "$kort_cache" "$kort_config"

  # =========================================================================
  # Setup: zsh-abbr (session abbreviations for fast path)
  # =========================================================================
  typeset -gA ABBR_REGULAR_SESSION_ABBREVIATIONS
  ABBR_REGULAR_SESSION_ABBREVIATIONS=()  # clear

  for i in $(seq 0 $((SIZE - 1))); do
    local kw="abbr${i}"
    ABBR_REGULAR_SESSION_ABBREVIATIONS[${(qqq)kw}]="expanded command ${i} with some arguments"
  done

  # =========================================================================
  # Benchmark 1: kort expand (external process)
  # =========================================================================
  bench_start=$EPOCHREALTIME
  for _iter in $(seq 1 $ITERATIONS); do
    $KORT_BIN expand --lbuffer "$target_keyword" --rbuffer "" --cache "$kort_cache" --config "$kort_config" >/dev/null
  done
  bench_end=$EPOCHREALTIME
  kort_total=$(( bench_end - bench_start ))
  results_kort[$SIZE]=$kort_total

  print_row "kort expand" $kort_total $ITERATIONS

  # =========================================================================
  # Benchmark 1b: kort serve (coproc pipe communication)
  # =========================================================================
  # Start serve process
  serve_in="${kort_dir}/serve_in"
  serve_out="${kort_dir}/serve_out"
  mkfifo "$serve_in" "$serve_out" 2>/dev/null || true
  $KORT_BIN serve --cache "$kort_cache" --config "$kort_config" < "$serve_in" > "$serve_out" 2>/dev/null &
  serve_pid=$!
  exec {serve_fd_w}>"$serve_in"
  exec {serve_fd_r}<"$serve_out"

  # Warmup serve (5 pings)
  for _w in {1..5}; do
    echo "ping" >&$serve_fd_w
    while read -r _sline <&$serve_fd_r; do
      [[ $_sline == $'\x1e'* ]] && break
    done
  done

  bench_start=$EPOCHREALTIME
  for _iter in $(seq 1 $ITERATIONS); do
    echo "expand\t${target_keyword}\t" >&$serve_fd_w
    while read -r _sline <&$serve_fd_r; do
      [[ $_sline == $'\x1e'* ]] && break
    done
  done
  bench_end=$EPOCHREALTIME
  kort_serve_total=$(( bench_end - bench_start ))
  results_kort_serve[$SIZE]=$kort_serve_total

  # Cleanup serve process
  exec {serve_fd_w}>&-
  exec {serve_fd_r}<&-
  wait $serve_pid 2>/dev/null
  rm -f "$serve_in" "$serve_out"

  print_row_compare "kort serve (coproc)" $kort_serve_total $ITERATIONS $kort_total

  # =========================================================================
  # Benchmark 2: zsh-abbr expand-line (full function path)
  # =========================================================================
  bench_start=$EPOCHREALTIME
  for _iter in $(seq 1 $ITERATIONS); do
    typeset -A reply
    abbr-expand-line "$target_keyword" "" >/dev/null 2>&1
  done
  bench_end=$EPOCHREALTIME
  abbr_total=$(( bench_end - bench_start ))
  results_abbr[$SIZE]=$abbr_total

  print_row_compare "zsh-abbr expand-line" $abbr_total $ITERATIONS $kort_total

  # =========================================================================
  # Benchmark 3: raw zsh associative array lookup (lower bound)
  # =========================================================================
  quoted_target="${(qqq)target_keyword}"
  bench_start=$EPOCHREALTIME
  for _iter in $(seq 1 $ITERATIONS); do
    _exp=${ABBR_REGULAR_SESSION_ABBREVIATIONS[$quoted_target]}
  done
  bench_end=$EPOCHREALTIME
  raw_total=$(( bench_end - bench_start ))
  results_raw[$SIZE]=$raw_total

  print_row_compare "raw zsh hash lookup" $raw_total $ITERATIONS $kort_total
done

# =============================================================================
# Summary table
# =============================================================================
print_header "Summary (µs/op)"

printf '  %-12s %12s %18s %18s %18s\n' "Abbr Count" "kort" "kort serve" "zsh-abbr" "raw zsh lookup"
printf '  %-12s %12s %18s %18s %18s\n' "──────────" "──────────" "────────────────" "────────────────" "────────────────"

for SIZE in $SIZES; do
  local k_us=$(( ${results_kort[$SIZE]} / $ITERATIONS * 1000000 ))
  local ks_us=$(( ${results_kort_serve[$SIZE]} / $ITERATIONS * 1000000 ))
  local a_us=$(( ${results_abbr[$SIZE]} / $ITERATIONS * 1000000 ))
  local r_us=$(( ${results_raw[$SIZE]} / $ITERATIONS * 1000000 ))
  local ks_ratio=$(( ${results_kort_serve[$SIZE]} / ${results_kort[$SIZE]} ))
  local a_ratio=$(( ${results_abbr[$SIZE]} / ${results_kort[$SIZE]} ))
  local r_ratio=$(( ${results_raw[$SIZE]} / ${results_kort[$SIZE]} ))
  printf '  %-12d %9.1f µs %11.1f µs (%.2fx) %11.1f µs (%.2fx) %11.1f µs (%.2fx)\n' \
    $SIZE $k_us $ks_us $ks_ratio $a_us $a_ratio $r_us $r_ratio
done

# =============================================================================
# Additional: hyperfine comparison (if available)
# =============================================================================
if command -v hyperfine >/dev/null 2>&1; then
  print_header "hyperfine: kort expand (500 abbreviations, precise measurement)"

  local kort_dir_500="${BENCH_TMPDIR}/kort_500"
  local kort_config_500="${kort_dir_500}/config/kort.toml"
  local kort_cache_500="${kort_dir_500}/cache/kort/kort.cache"

  hyperfine \
    --warmup 100 \
    --min-runs 500 \
    --shell=none \
    -n "kort expand (500 abbrs)" \
    "$KORT_BIN expand --lbuffer abbr250 --rbuffer '' --cache $kort_cache_500 --config $kort_config_500" \
    2>&1

  print_header "hyperfine: kort expand vs zsh startup+lookup (100 abbreviations)"

  local kort_dir_100="${BENCH_TMPDIR}/kort_100"
  local kort_config_100="${kort_dir_100}/config/kort.toml"
  local kort_cache_100="${kort_dir_100}/cache/kort/kort.cache"

  # Create a self-contained zsh script for zsh-abbr benchmarking
  local abbr_bench_script="${BENCH_TMPDIR}/abbr_bench.zsh"
  {
    echo '#!/usr/bin/env zsh'
    echo 'typeset -gA ABBR_REGULAR_SESSION_ABBREVIATIONS'
    for i in $(seq 0 99); do
      local kw="abbr${i}"
      echo "ABBR_REGULAR_SESSION_ABBREVIATIONS[${(qqq)kw}]=\"expanded command ${i} with some arguments\""
    done
    echo 'local _exp=${ABBR_REGULAR_SESSION_ABBREVIATIONS["abbr50"]}'
  } > "$abbr_bench_script"
  chmod +x "$abbr_bench_script"

  hyperfine \
    --warmup 20 \
    --min-runs 200 \
    -n "kort expand (100 abbrs)" \
    "$KORT_BIN expand --lbuffer abbr50 --rbuffer '' --cache $kort_cache_100 --config $kort_config_100" \
    -n "zsh: raw hash lookup (100 abbrs, includes zsh startup)" \
    "zsh $abbr_bench_script" \
    2>&1
fi

printf '\n✓ Benchmark complete.\n'
