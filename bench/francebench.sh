#!/usr/bin/env bash
# The France comparison on the Mac mini: tileroute, OSRM MLD, OSRM CH, GraphHopper CH, one at a time, the same 2,000
# random pairs. Run from ~/remote/tr-france, which holds: the tileroute source in src-tr/ (built), guard.sh.
# The input stays in ~/remote/data/france-latest.osm.pbf. Each router's files are deleted after its run.
# Output: results/*.txt (preparation and queries), results/*.times (one line for each query: seconds, duration, URL).
set -uo pipefail
export PATH=$HOME/.cargo/bin:/opt/homebrew/bin:/opt/homebrew/opt/openjdk/bin:$PATH
cd "$(dirname "$0")"
PBF=$HOME/remote/data/france-latest.osm.pbf
TR=src-tr/target/release/tileroute
TRARGS=${TRARGS:-}  # more build options, for example "--partition 16"
DUCKOSM=$HOME/remote/no-osmium-gdal/rust/target/release/duckosm
ONLY=${ONLY:-}
mkdir -p results work
step() { echo "== $(date +%H:%M:%S) $* (free: $(df -g / | awk 'NR==2{print $4}') GB)"; }
want() { [ -z "$ONLY" ] || [[ " $ONLY " == *" $1 "* ]]; }
need() {  # stop when less than 30 GB would stay free
  local free; free=$(df -g / | awk 'NR==2{print $4}')
  if [ "$free" -lt 32 ]; then echo "STOP: only $free GB free"; exit 1; fi
}
# measure NAME CMD...: wall time and peak memory, under the disk guard
measure() {
  local name=$1; shift
  need
  /usr/bin/time -l ./guard.sh "$@" > "work/$name.log" 2> "work/$name.time" || { echo "FAILED: $name"; tail -20 "work/$name.log" "work/$name.time"; exit 1; }
  awk -v n="$name" '/ real / {t=$1} /maximum resident set size/ {m=$1} END {printf "%s: %.1f s, peak %.0f MB\n", n, t, m/1048576}' "work/$name.time" | tee -a results/prepare.txt
}
wait_http() {
  for _ in $(seq 1 900); do
    code=$(curl -s -o /dev/null -w "%{http_code}" "$1" || true)
    [ "$code" != "000" ] && return 0
    sleep 1
  done
  echo "no server at $1"; exit 1
}
# run_queries NAME PASS KEY DIV: the requests in work/NAME.curl, one after another; the route duration is the first number
# after "KEY": divided by DIV
run_queries() {
  local name=$1 pass=$2 key=$3 div=$4
  curl -s --config "work/$name.curl" > "work/$name.out"
  awk -v key="\"$key\":" -v div="$div" 'BEGIN{RS="~~"}
    NR>1 {split($0,a,"\n"); split(a[1],f," "); if (!(f[2] in seen)) {seen[f[2]]=1; print f[1], d, f[2]}}
    {d="null"; i=index($0,key); if (i) {r=substr($0,i+length(key)); sub(/^[ ]*/,"",r); match(r,/^[0-9.]+/); if (RLENGTH>0) d=substr(r,1,RLENGTH)/div}}' \
    "work/$name.out" > "results/$name.$pass.times"
  awk -v n="$name $pass" '{s+=$1; k++; if ($2=="null") z++} END {printf "%s: %d queries, %.3f ms per query, %d with no route\n", n, k, s/k*1000, z}' "results/$name.$pass.times" | tee -a results/queries.txt
}
rss() { ps -o rss= -p "$1" | awk '{printf "%.0f MB", $1/1024}'; }

# ---------------------------------------------------------------- the France database and tileroute
if want tileroute; then
  step "duckOSM: France driving"
  measure duckosm "$DUCKOSM" "$PBF" -o work --name france -f duckdb -m driving --no-ways-tables --no-bus-routes --levels none
  measure slim $TR slim work/france.duckdb work/france-slim.duckdb
  rm -f work/france.duckdb
  step "tileroute build --by-tile"
  measure tileroute-build $TR build work/france-slim.duckdb -o work/tiles --by-tile $TRARGS
  du -sh work/tiles | sed 's/^/tileroute files: /' | tee -a results/prepare.txt
  step "tileroute queries: 2,000 random pairs, one thread, a cold and a warm pass"
  /usr/bin/time -l $TR route work/tiles ${ROUTEARGS:---random 2000 --write results/pairs.csv} > results/tileroute-route.txt 2> work/route.time
  cat results/tileroute-route.txt | tee -a results/queries.txt
  grep "maximum resident" work/route.time | tee -a results/queries.txt
  $TR points work/france-slim.duckdb results/pairs.csv results/points.csv
  rm -rf work/tiles
fi

# ---------------------------------------------------------------- OSRM: MLD, then CH (each on its own extract)
PROFILES=$(brew --prefix osrm-backend)/share/osrm/profiles
for algo in mld ch; do
  want "osrm-$algo" || continue
  step "OSRM $algo"
  mkdir -p work/osrm-$algo
  ln -sf "$PBF" work/osrm-$algo/france.osm.pbf
  measure osrm-$algo-extract osrm-extract -p "$PROFILES/car.lua" work/osrm-$algo/france.osm.pbf
  if [ $algo = mld ]; then
    measure osrm-partition osrm-partition work/osrm-mld/france.osrm
    measure osrm-customize osrm-customize work/osrm-mld/france.osrm
  else
    measure osrm-contract osrm-contract work/osrm-ch/france.osrm
  fi
  du -sh work/osrm-$algo | sed "s/^/osrm $algo files: /" | tee -a results/prepare.txt
  osrm-routed --algorithm $algo --port 15000 work/osrm-$algo/france.osrm > work/osrm-routed-$algo.log 2>&1 &
  pid=$!
  wait_http "http://localhost:15000/route/v1/driving/2.35,48.85;2.36,48.86"
  tail -n +2 results/points.csv | awk -F, '{printf "url = \"http://localhost:15000/route/v1/driving/%s,%s;%s,%s?overview=false\"\nwrite-out = \"~~%%{time_total} %%{url_effective}\\n\"\n", $4,$5,$6,$7}' > work/osrm-$algo.curl
  run_queries osrm-$algo cold duration 1
  run_queries osrm-$algo warm duration 1
  echo "osrm-$algo server: $(rss $pid) (after the queries)" | tee -a results/queries.txt
  kill $pid; wait $pid 2>/dev/null
  rm -rf work/osrm-$algo
done

# ---------------------------------------------------------------- GraphHopper CH
if want gh; then
  step "GraphHopper CH"
  [ -f graphhopper-web-11.1.jar ] || curl -sSLO https://github.com/graphhopper/graphhopper/releases/download/11.1/graphhopper-web-11.1.jar
  cat > work/gh.yml <<EOF
graphhopper:
  datareader.file: $PBF
  graph.location: work/gh-graph
  import.osm.ignored_highways: footway,cycleway,path,pedestrian,steps
  profiles:
    - name: car
      custom_model_files: [car.json]
  profiles_ch:
    - profile: car
  graph.encoded_values: car_access, car_average_speed, road_access
server:
  application_connectors:
    - type: http
      port: 8989
  admin_connectors:
    - type: http
      port: 8990
EOF
  measure gh-import java -Xmx12g -jar graphhopper-web-11.1.jar import work/gh.yml
  du -sh work/gh-graph | sed 's/^/gh files: /' | tee -a results/prepare.txt
  java -Xmx12g -jar graphhopper-web-11.1.jar server work/gh.yml > work/gh-server.log 2>&1 &
  pid=$!
  wait_http "http://localhost:8989/health"
  tail -n +2 results/points.csv | awk -F, '{printf "url = \"http://localhost:8989/route?point=%s,%s&point=%s,%s&profile=car&calc_points=false&instructions=false\"\nwrite-out = \"~~%%{time_total} %%{url_effective}\\n\"\n", $5,$4,$7,$6}' > work/gh-ch.curl
  run_queries gh-ch cold time 1000
  run_queries gh-ch warm time 1000
  echo "gh server: $(rss $pid) (after the queries)" | tee -a results/queries.txt
  kill $pid; wait $pid 2>/dev/null
  rm -rf work/gh-graph
fi
step "done"
