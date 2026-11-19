#!/usr/bin/env bash
# The same-computer comparison (milestone M2b): OSRM (CH, MLD), GraphHopper (CH, LM), Valhalla, on the Mac mini.
# One router at a time. Run from ~/remote/router-bench, which holds:
#   kentucky-latest.osm.pbf, points.csv (from tileroute: source,target,cost,lon1,lat1,lon2,lat2,target_cost),
#   graphhopper-web-11.1.jar, venv/ (pyvalhalla).
# Output: results/<router>.txt (preparation time and peak memory), results/<router>.times (one line for each query:
# curl total time in seconds, and the route duration in seconds or "null").
#
# The queries go over HTTP on localhost with one kept connection (curl --config), one at a time: the time includes a
# small HTTP cost (about 0.1 ms).
set -euo pipefail
export PATH=/opt/homebrew/bin:/opt/homebrew/opt/openjdk/bin:$PATH
cd "$(dirname "$0")"
PBF=kentucky-latest.osm.pbf
mkdir -p results work
step() { echo "== $(date +%H:%M:%S) $*"; }
# measure NAME CMD...: wall time and peak memory of one preparation command
measure() {
  local name=$1; shift
  /usr/bin/time -l "$@" > "work/$name.log" 2> "work/$name.time" || { tail -20 "work/$name.log" "work/$name.time"; exit 1; }
  awk -v n="$name" '/real/ {t=$1} /maximum resident set size/ {m=$1} END {printf "%s: %.1f s, peak %.0f MB\n", n, t, m/1048576}' "work/$name.time" | tee -a "results/prepare.txt"
}
# wait_http URL: until the server answers
wait_http() {
  for _ in $(seq 1 600); do
    code=$(curl -s -o /dev/null -w "%{http_code}" "$1" || true)
    [ "$code" != "000" ] && return 0
    sleep 1
  done
  echo "no server at $1"; exit 1
}
# run_queries NAME KEY DIV: the curl config in work/NAME.curl, one request after another; the route duration is the
# first JSON number after "KEY":, divided by DIV (to seconds)
run_queries() {
  local name=$1 key=$2 div=$3
  curl -s --config "work/$name.curl" > "work/$name.out"
  # each response is followed by "~~<time_total>"; a record of RS="~~" is: the time of the previous response, then
  # the JSON of the next one
  awk -v key="\"$key\":" -v div="$div" 'BEGIN{RS="~~"}
    NR>1 {split($0,a,"\n"); split(a[1],f," "); if (!(f[2] in seen)) {seen[f[2]]=1; print f[1], d, f[2]}}
    {d="null"; i=index($0,key); if (i) {r=substr($0,i+length(key)); sub(/^[ ]*/,"",r); match(r,/^[0-9.]+/); if (RLENGTH>0) d=substr(r,1,RLENGTH)/div}}' \
    "work/$name.out" > "results/$name.times"
  awk -v n="$name" '{s+=$1; k++; if ($2=="null") z++} END {printf "%s: %d queries, %.3f ms per query, %d with no route\n", n, k, s/k*1000, z}' "results/$name.times" | tee -a results/queries.txt
  # stop if most queries have no route: the server did not answer as it should
  local bad; bad=$(awk '$2=="null"' "results/$name.times" | wc -l)
  if [ "$bad" -gt 2000 ]; then echo "$name: $bad queries with no route, stop"; head -c 600 "work/$name.out"; exit 1; fi
}

# ---------------------------------------------------------------- OSRM
step "OSRM extract, CH, MLD"
want() { [ -z "${ONLY:-}" ] || [[ " $ONLY " == *" $1 "* ]]; }
if want osrm; then
PROFILES=$(brew --prefix osrm-backend)/share/osrm/profiles
cp "$PBF" work/osrm.osm.pbf
measure osrm-extract osrm-extract -p "$PROFILES/car.lua" work/osrm.osm.pbf
# CH and MLD need their own copies of the files
mkdir -p work/ch work/mld
cp work/osrm.osrm* work/ch/
mv work/osrm.osrm* work/mld/
measure osrm-contract osrm-contract work/ch/osrm.osrm
measure osrm-partition osrm-partition work/mld/osrm.osrm
measure osrm-customize osrm-customize work/mld/osrm.osrm
for algo in ch mld; do
  step "OSRM $algo queries"
  osrm-routed --algorithm "$algo" --port 15000 "work/$algo/osrm.osrm" > "work/osrm-routed-$algo.log" 2>&1 &
  pid=$!
  wait_http "http://localhost:15000/route/v1/driving/-84.5,38.0;-84.4,38.1"
  tail -n +2 points.csv | awk -F, '{printf "url = \"http://localhost:15000/route/v1/driving/%s,%s;%s,%s?overview=false\"\nwrite-out = \"~~%%{time_total} %%{url_effective}\\n\"\n", $4,$5,$6,$7}' > "work/osrm-$algo.curl"
  run_queries "osrm-$algo" duration 1
  echo "osrm-$algo server: peak $(ps -o rss= -p $pid | awk '{printf "%.0f MB", $1/1024}') (at the end)" | tee -a results/queries.txt
  kill $pid; wait $pid 2>/dev/null || true
done
du -sh work/ch | sed 's/^/osrm CH files: /' | tee -a results/prepare.txt
du -sh work/mld | sed 's/^/osrm MLD files: /' | tee -a results/prepare.txt
rm -rf work/ch work/mld work/osrm.osm.pbf
fi

# ---------------------------------------------------------------- GraphHopper
if want gh; then
step "GraphHopper import (CH and LM)"
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
  profiles_lm:
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
measure gh-import java -Xmx8g -jar graphhopper-web-11.1.jar import work/gh.yml
java -Xmx8g -jar graphhopper-web-11.1.jar server work/gh.yml > work/gh-server.log 2>&1 &
pid=$!
wait_http "http://localhost:8989/health"
for mode in ch lm; do
  step "GraphHopper $mode queries"
  extra=""; [ $mode = lm ] && extra="&ch.disable=true"
  tail -n +2 points.csv | awk -F, -v x="$extra" '{printf "url = \"http://localhost:8989/route?point=%s,%s&point=%s,%s&profile=car&calc_points=false&instructions=false%s\"\nwrite-out = \"~~%%{time_total} %%{url_effective}\\n\"\n", $5,$4,$7,$6,x}' > "work/gh-$mode.curl"
  run_queries "gh-$mode" time 1000
done
echo "gh server: $(ps -o rss= -p $pid | awk '{printf "%.0f MB", $1/1024}') (at the end)" | tee -a results/queries.txt
kill $pid; wait $pid 2>/dev/null || true
du -sh work/gh-graph | sed 's/^/gh files: /' | tee -a results/prepare.txt
rm -rf work/gh-graph
fi

# ---------------------------------------------------------------- Valhalla
if want valhalla; then
step "Valhalla tiles"
V=venv/bin
# the programs of pyvalhalla; its wrapper scripts in venv/bin do not find them
VB=$(ls -d venv/lib/python3*/site-packages/valhalla/bin)
$V/valhalla_build_config --mjolnir-tile-dir "$PWD/work/valhalla_tiles" --mjolnir-tile-extract "$PWD/work/valhalla_tiles.tar" \
  --mjolnir-timezone "" --mjolnir-admin "" > work/valhalla.json
measure valhalla-tiles $VB/valhalla_build_tiles -c work/valhalla.json "$PBF"
$VB/valhalla_service work/valhalla.json 1 > work/valhalla.log 2>&1 &
pid=$!
wait_http "http://localhost:8002/status"
step "Valhalla queries"
# GET with the request as URL-encoded JSON (%7B { %7D } %22 " %5B [ %5D ] %3A : %2C ,)
tail -n +2 points.csv | awk -F, '{printf "url = \"http://localhost:8002/route?json=%%7B%%22locations%%22%%3A%%5B%%7B%%22lat%%22%%3A%s%%2C%%22lon%%22%%3A%s%%7D%%2C%%7B%%22lat%%22%%3A%s%%2C%%22lon%%22%%3A%s%%7D%%5D%%2C%%22costing%%22%%3A%%22auto%%22%%2C%%22directions_type%%22%%3A%%22none%%22%%7D\"\nwrite-out = \"~~%%{time_total} %%{url_effective}\\n\"\n", $5,$4,$7,$6}' > work/valhalla.curl
run_queries valhalla time 1
echo "valhalla server: $(ps -o rss= -p $pid | awk '{printf "%.0f MB", $1/1024}') (at the end)" | tee -a results/queries.txt
kill $pid; wait $pid 2>/dev/null || true
du -sh work/valhalla_tiles | sed 's/^/valhalla files: /' | tee -a results/prepare.txt
rm -rf work/valhalla_tiles work/valhalla_tiles.tar
fi
step "done"
