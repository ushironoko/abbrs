#!/usr/bin/env zsh
# =============================================================================
# abbrs vs zsh-abbr comparison benchmark
# =============================================================================
# Measures end-to-end expansion latency as experienced by the user:
#   - abbrs:    fork+exec `abbrs expand` → cache read → HashMap lookup → stdout
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
ABBRS_BIN=${PROJECT_ROOT}/target/release/abbrs
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
# Build abbrs
# ---------------------------------------------------------------------------
if [[ ! -x $ABBRS_BIN ]]; then
  echo "Building abbrs (release)..."
  (cd $PROJECT_ROOT && cargo build --release 2>&1)
fi
echo "abbrs binary: $ABBRS_BIN"
echo "Iterations per measurement: $ITERATIONS"

# ---------------------------------------------------------------------------
# Load zsh-abbr (non-interactive, bindings disabled)
# ---------------------------------------------------------------------------
ABBR_DEFAULT_BINDINGS=0
ABBR_AUTOLOAD=0
ABBR_GET_AVAILABLE_ABBREVIATION=0
source /opt/homebrew/share/zsh-abbr/zsh-abbr.zsh 2>/dev/null || true

# ---------------------------------------------------------------------------
# Warmup abbrs binary (page cache)
# ---------------------------------------------------------------------------
warmup_abbrs() {
  local cache_path=$1
  local config_path=$2
  for _w in {1..5}; do
    $ABBRS_BIN expand --lbuffer "warmup" --rbuffer "" --cache "$cache_path" --config "$config_path" >/dev/null 2>&1 || true
  done
}

# =============================================================================
# Main benchmark loop
# =============================================================================
printf '\n'
printf '╔══════════════════════════════════════════════════════════════════════╗\n'
printf '║           abbrs vs zsh-abbr  Expansion Benchmark                    ║\n'
printf '╚══════════════════════════════════════════════════════════════════════╝\n'

# Collect results for summary table
typeset -A results_abbrs results_abbrs_serve results_abbr results_raw

# Pre-declare loop variables to avoid zsh typeset output on re-declaration
local bench_start bench_end
local target_keyword abbrs_dir abbrs_config abbrs_cache
local abbrs_total abbrs_serve_total abbr_total raw_total quoted_target
local serve_in serve_out serve_pid serve_fd_w serve_fd_r _sline

for SIZE in $SIZES; do
  print_header "Abbreviation count: $SIZE"

  target_keyword="abbr$((SIZE / 2))"

  # =========================================================================
  # Setup: abbrs
  # =========================================================================
  abbrs_dir="${BENCH_TMPDIR}/abbrs_${SIZE}"
  mkdir -p "${abbrs_dir}/config" "${abbrs_dir}/cache/abbrs"
  abbrs_config="${abbrs_dir}/config/abbrs.toml"
  abbrs_cache="${abbrs_dir}/cache/abbrs/abbrs.cache"

  # Generate abbrs.toml
  {
    echo '[settings]'
    echo ''
    for i in $(seq 0 $((SIZE - 1))); do
      echo "[[abbr]]"
      echo "keyword = \"abbr${i}\""
      echo "expansion = \"expanded command ${i} with some arguments\""
      echo ""
    done
  } > "$abbrs_config"

  # Compile (set XDG paths so cache goes where we want)
  XDG_CONFIG_HOME="${abbrs_dir}/config" XDG_CACHE_HOME="${abbrs_dir}/cache" \
    $ABBRS_BIN compile --config "$abbrs_config" 2>/dev/null

  # Verify cache exists
  if [[ ! -f "$abbrs_cache" ]]; then
    echo "ERROR: abbrs cache not created at $abbrs_cache"
    ls -la "${abbrs_dir}/cache/" 2>&1
    continue
  fi

  # Warmup
  warmup_abbrs "$abbrs_cache" "$abbrs_config"

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
  # Benchmark 1: abbrs expand (external process)
  # =========================================================================
  bench_start=$EPOCHREALTIME
  for _iter in $(seq 1 $ITERATIONS); do
    $ABBRS_BIN expand --lbuffer "$target_keyword" --rbuffer "" --cache "$abbrs_cache" --config "$abbrs_config" >/dev/null
  done
  bench_end=$EPOCHREALTIME
  abbrs_total=$(( bench_end - bench_start ))
  results_abbrs[$SIZE]=$abbrs_total

  print_row "abbrs expand" $abbrs_total $ITERATIONS

  # =========================================================================
  # Benchmark 1b: abbrs serve (coproc pipe communication)
  # =========================================================================
  # Start serve process
  serve_in="${abbrs_dir}/serve_in"
  serve_out="${abbrs_dir}/serve_out"
  mkfifo "$serve_in" "$serve_out" 2>/dev/null || true
  $ABBRS_BIN serve --cache "$abbrs_cache" --config "$abbrs_config" < "$serve_in" > "$serve_out" 2>/dev/null &
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
  abbrs_serve_total=$(( bench_end - bench_start ))
  results_abbrs_serve[$SIZE]=$abbrs_serve_total

  # Cleanup serve process
  exec {serve_fd_w}>&-
  exec {serve_fd_r}<&-
  wait $serve_pid 2>/dev/null
  rm -f "$serve_in" "$serve_out"

  print_row_compare "abbrs serve (coproc)" $abbrs_serve_total $ITERATIONS $abbrs_total

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

  print_row_compare "zsh-abbr expand-line" $abbr_total $ITERATIONS $abbrs_total

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

  print_row_compare "raw zsh hash lookup" $raw_total $ITERATIONS $abbrs_total
done

# =============================================================================
# Summary table
# =============================================================================
print_header "Summary (µs/op)"

printf '  %-12s %12s %18s %18s %18s\n' "Abbr Count" "abbrs" "abbrs serve" "zsh-abbr" "raw zsh lookup"
printf '  %-12s %12s %18s %18s %18s\n' "──────────" "──────────" "────────────────" "────────────────" "────────────────"

for SIZE in $SIZES; do
  local k_us=$(( ${results_abbrs[$SIZE]} / $ITERATIONS * 1000000 ))
  local ks_us=$(( ${results_abbrs_serve[$SIZE]} / $ITERATIONS * 1000000 ))
  local a_us=$(( ${results_abbr[$SIZE]} / $ITERATIONS * 1000000 ))
  local r_us=$(( ${results_raw[$SIZE]} / $ITERATIONS * 1000000 ))
  local ks_ratio=$(( ${results_abbrs_serve[$SIZE]} / ${results_abbrs[$SIZE]} ))
  local a_ratio=$(( ${results_abbr[$SIZE]} / ${results_abbrs[$SIZE]} ))
  local r_ratio=$(( ${results_raw[$SIZE]} / ${results_abbrs[$SIZE]} ))
  printf '  %-12d %9.1f µs %11.1f µs (%.2fx) %11.1f µs (%.2fx) %11.1f µs (%.2fx)\n' \
    $SIZE $k_us $ks_us $ks_ratio $a_us $a_ratio $r_us $r_ratio
done

# =============================================================================
# Additional: hyperfine comparison (if available)
# =============================================================================
if command -v hyperfine >/dev/null 2>&1; then
  print_header "hyperfine: abbrs expand (500 abbreviations, precise measurement)"

  local abbrs_dir_500="${BENCH_TMPDIR}/abbrs_500"
  local abbrs_config_500="${abbrs_dir_500}/config/abbrs.toml"
  local abbrs_cache_500="${abbrs_dir_500}/cache/abbrs/abbrs.cache"

  hyperfine \
    --warmup 100 \
    --min-runs 500 \
    --shell=none \
    -n "abbrs expand (500 abbrs)" \
    "$ABBRS_BIN expand --lbuffer abbr250 --rbuffer '' --cache $abbrs_cache_500 --config $abbrs_config_500" \
    2>&1

  print_header "hyperfine: abbrs expand vs zsh startup+lookup (100 abbreviations)"

  local abbrs_dir_100="${BENCH_TMPDIR}/abbrs_100"
  local abbrs_config_100="${abbrs_dir_100}/config/abbrs.toml"
  local abbrs_cache_100="${abbrs_dir_100}/cache/abbrs/abbrs.cache"

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
    -n "abbrs expand (100 abbrs)" \
    "$ABBRS_BIN expand --lbuffer abbr50 --rbuffer '' --cache $abbrs_cache_100 --config $abbrs_config_100" \
    -n "zsh: raw hash lookup (100 abbrs, includes zsh startup)" \
    "zsh $abbr_bench_script" \
    2>&1
fi

printf '\n✓ Benchmark complete.\n'
