#!/usr/bin/env bash
# Validate real responses of the R4 and R5 routes with the official HL7 FHIR validator (NEW-9, NEW-10, QUAL-39).
#
#   scripts/validate-fhir.sh            needs DATABASE_URL (a migrated PostgreSQL) and Java 17 or later
#
# The validator is pinned to one release and its SHA-256 (the value GitHub publishes for the release asset),
# downloaded to $PAS_VALIDATOR_DIR (default ~/.cache/pas-fhir-validator) and checked before it is run. Packages
# (FHIR R4 and R5 core, terminology, the IHE profiles) are fetched by the validator from packages.fhir.org into
# ~/.fhir/packages. Exit status is the validator's: a profile error fails the script.
set -euo pipefail

VERSION=6.10.4
SHA256=1106b9d58f9e363e47bea7c4fc065841e5fc91fe9d062775c3bfdd212bd653cc
DIR="${PAS_VALIDATOR_DIR:-$HOME/.cache/pas-fhir-validator}"
JAR="$DIR/validator_cli-$VERSION.jar"
SAMPLES="$(mktemp -d)"
trap 'rm -rf "$SAMPLES"' EXIT

sha() { if command -v sha256sum >/dev/null; then sha256sum "$1" | cut -d' ' -f1; else shasum -a 256 "$1" | cut -d' ' -f1; fi; }

mkdir -p "$DIR"
if [ ! -f "$JAR" ] || [ "$(sha "$JAR")" != "$SHA256" ]; then
  curl -fsSL -o "$JAR" "https://github.com/hapifhir/org.hl7.fhir.core/releases/download/$VERSION/validator_cli.jar"
  [ "$(sha "$JAR")" = "$SHA256" ] || { echo "validator jar does not match the pinned SHA-256" >&2; rm -f "$JAR"; exit 1; }
fi

( cd "$(dirname "$0")/.." && \
  PAS_DUMP_DIR="$SAMPLES" HOSTNAME="${HOSTNAME:-pas-server}" \
  cargo test -p patient-administration-system --test fhir_samples_dump_test -- --ignored --test-threads=1 )

status=0
validate() { # profile-url, file
  java -jar "$JAR" "$2" -version 4.0.1 -ig ihe.iti.balp -ig ihe.iti.scheduling -tx n/a ${1:+-profile "$1"} || status=1
}
BALP=https://profiles.ihe.net/ITI/BALP/StructureDefinition/IHE.BasicAudit
validate "$BALP.PatientRead"   "$SAMPLES"/balp-*-R.json
validate "$BALP.PatientCreate" "$SAMPLES"/balp-*-C.json
validate "$BALP.PatientUpdate" "$SAMPLES"/balp-*-U.json
validate "$BALP.PatientDelete" "$SAMPLES"/balp-*-D.json
SCHED=https://profiles.ihe.net/ITI/Scheduling/StructureDefinition
for f in find hold book; do validate "$SCHED/ihe-sched-avail-bundle" "$SAMPLES/ihe-$f.json"; done
validate "" "$SAMPLES/ihe-metadata.json"
# The Wales exchange (NEW-1): fhir.r4.wales is built on UK Core STU2, which the validator fetches with it.
WALES=https://fhir.nhs.wales/StructureDefinition/DataStandardsWales
for kind in Appointment Encounter ServiceRequest; do
  java -jar "$JAR" "$SAMPLES/wales-$kind.json" -version 4.0.1 -ig fhir.r4.wales#2.4.0 -tx n/a \
    -profile "$WALES-$kind" || status=1
done
# The R5 endpoint (/fhir), against the base R5 resource definitions, no implementation guide (QUAL-39). Each read
# is mapped to R5 in `api/fhir/r5.rs`; every one must validate.
for f in metadata Appointment Encounter Slot Schedule; do
  java -jar "$JAR" "$SAMPLES/r5-$f.json" -version 5.0.0 -tx n/a || status=1
done
exit $status
