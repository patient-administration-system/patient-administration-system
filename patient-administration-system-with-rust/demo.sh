#!/usr/bin/env bash
# demo.sh: a walk through the headline ADT flow against a running server.
#
# Needs the server (default http://localhost:8080) on a database that `pas-seed` has filled with the demo ward and
# beds. The PAS holds no people: a patient is an id from person-service. Set PERSON_ID to a real one; without it
# the script uses a made-up id, which is enough for the bed movements (the PAS only stores the id) but means
# nothing for anything that reads the person from person-service (letters, patient search).
#
#   cargo run --bin pas-migrate -- up && cargo run --bin pas-seed && cargo run --release &
#   ./demo.sh

set -euo pipefail

BASE="${BASE:-http://localhost:8080}"
JQ="${JQ:-jq -C}"
JQR="jq -r"
PERSON_ID="${PERSON_ID:-$(uuidgen | tr 'A-Z' 'a-z')}"

say() { printf "\n\033[1;36m== %s ==\033[0m\n" "$*"; }
get() { curl -sS "$BASE$1"; }
post() { curl -sS -X POST "$BASE$1" -H 'content-type: application/json' -d "$2"; }

say "Health"
get /api/health | $JQ '.'

say "The demo ward"
FACILITY_ID=$(get /api/facilities | $JQR '.data[] | select(.code == "DEMO") | .id')
[ -n "$FACILITY_ID" ] || { echo "no DEMO facility: run pas-seed first" >&2; exit 1; }
WARD_ID=$(get "/api/facilities/$FACILITY_ID/wards" | $JQR '.data[] | select(.code == "WARD-A") | .id')
echo "facility_id=$FACILITY_ID ward_id=$WARD_ID person_id=$PERSON_ID"

say "Ward occupancy (before admit)"
get "/api/wards/$WARD_ID/occupancy" | $JQ '.'

say "Beds in the ward"
mapfile -t BED_IDS < <(get "/api/wards/$WARD_ID/beds" | $JQR '.data | sort_by(.code) | .[].id')
BED0=${BED_IDS[0]}
BED1=${BED_IDS[1]}
echo "bed0=$BED0 bed1=$BED1"

say "Admit the person to bed0"
ADMIT=$(post /api/admissions "{\"person_id\":\"$PERSON_ID\",\"bed_id\":\"$BED0\"}")
echo "$ADMIT" | $JQ '.'
ADMISSION_ID=$(echo "$ADMIT" | $JQR '.data.admission.id')

say "Ward occupancy (after admit)"
get "/api/wards/$WARD_ID/occupancy" | $JQ '.'

say "Transfer to bed1"
post "/api/admissions/$ADMISSION_ID/transfer" "{\"new_bed_id\":\"$BED1\"}" | $JQ '.'

say "Discharge"
post "/api/admissions/$ADMISSION_ID/discharge" '{}' | $JQ '.'

say "Ward occupancy (after discharge)"
get "/api/wards/$WARD_ID/occupancy" | $JQ '.'

say "What was recorded about the admission"
get "/api/audit/entity?entity_type=admission&entity_id=$ADMISSION_ID" | $JQ '.data | map({action, entity_type, at})'

printf "\n\033[1;32mdone\033[0m\n"
