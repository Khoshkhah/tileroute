#!/usr/bin/env bash
# France with many metrics on the Mac mini: duckOSM, slim, then the structure with no costs and the metrics, one unit
# at a time; queries for each metric, the weighted query against two exact mixes, and 100 pairs against duckOSM's
# router. Run from ~/remote/tr-france, which holds: src-tr/ (built), guard.sh, pairs-in.csv (source,target,... with a
# header line). All temporary files stay in this folder; delete the folder at the end.
set -uo pipefail
export PATH=$HOME/.cargo/bin:/opt/homebrew/bin:$PATH
cd "$(dirname "$0")"
export TMPDIR=$PWD/tmp GUARD_GB=${GUARD_GB:-25}
mkdir -p tmp results work
PBF=$HOME/remote/data/france-latest.osm.pbf
TR=src-tr/target/release/tileroute
DUCKOSM=$HOME/remote/no-osmium-gdal/rust/target/release/duckosm
METRICS=${METRICS:-time,length,mix50,mix30}
step() { echo "== $(date +%H:%M:%S) $* (free: $(df -g / | awk 'NR==2{print $4}') GB)"; }
measure() {  # measure NAME CMD...: wall time and peak memory, under the disk guard
  local name=$1; shift
  /usr/bin/time -l ./guard.sh "$@" > "work/$name.log" 2> "work/$name.time" || { echo "FAILED: $name"; tail -20 "work/$name.log" "work/$name.time"; exit 1; }
  awk -v n="$name" '/ real / {t=$1} /maximum resident set size/ {m=$1} END {printf "%s: %.1f s, peak %.0f MB\n", n, t, m/1048576}' "work/$name.time" | tee -a results/prepare.txt
}
step "duckOSM: France driving"
measure duckosm "$DUCKOSM" "$PBF" -o work --name france -f duckdb -m driving --no-ways-tables --no-bus-routes --levels none
measure slim $TR slim work/france.duckdb work/france-slim.duckdb
rm -f work/france.duckdb
step "build: structure and metrics $METRICS, unit by unit"
measure cch-build $TR build work/france-slim.duckdb -o work/cch --by-tile --partition 16 --xyz-files --cch --metrics "$METRICS"
cat work/cch-build.log | tee -a results/prepare.txt
du -sh work/cch/* | tee -a results/prepare.txt
for m in ${METRICS//,/ }; do
  step "queries: metric $m"
  $TR route work/cch --metric "$m" --pairs-file pairs-in.csv --write "results/route-$m.csv" 2>&1 | tee -a results/queries.txt
done
step "weighted query"
$TR weigh work/cch --pairs-file pairs-in.csv --candidates time,length,mix50 --exact mix30 2>&1 | tee -a results/weigh.txt
$TR weigh work/cch --pairs-file pairs-in.csv --candidates time,length --exact mix30 2>&1 | grep "^l =" | tee -a results/weigh.txt
step "reference: 100 pairs with duckOSM's router"
$TR reference work/france-slim.duckdb pairs-in.csv results/reference.csv --max 100 2>&1 | tee -a results/reference.txt
python3 - <<'PY' | tee -a results/reference.txt
import csv
r = {(a, b): float(c) for a, b, c in list(csv.reader(open('results/reference.csv')))[1:]}
f = {(a, b): float(c) for a, b, c in list(csv.reader(open('results/route-time.csv')))[1:]}
k = [x for x in r if x in f]
eq = sum(r[x] == f[x] for x in k)
print(f"reference against the files (metric time): {len(k)} pairs, {eq} equal, {len(k) - eq} different")
PY
step "done"
