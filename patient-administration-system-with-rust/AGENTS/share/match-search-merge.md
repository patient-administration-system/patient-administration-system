# Match / Search / Merge

Person matching, search, dedup and merge all live in `person-service`, and the same for workers in `worker-service`. The PAS holds no matching code, search index or merge tombstones, and no `patients` table: it stores an opaque `person_id`.

## What the PAS does

- **Search:** `GET /api/patients/search` passes through to `person-service`.
- **HL7 v2 ADT:** a message's PID is resolved to a person through `person-service`, which is where a repeat MRN is recognised. Without the service the message gets an `AE` ACK.
- **Merge, from `person-service`:** `POST /api/upstream/person-merged` is a signed webhook. It needs `PERSON_SERVICE_WEBHOOK_SECRET`, refuses every call without it, and moves the PAS's own `person_id` columns to the survivor. `POST /api/upstream/worker-merged` is the same for workers.
- **Merge, from HL7 v2:** `POST /api/hl7/v2/merge` takes an ADT^A40 with the survivor in PID and the source MRN in MRG-1.1, resolves both through `person-service` and asks it to merge.

## Out of scope for the PAS

Probabilistic and phonetic matching, duplicate detection, and deciding which record survives a merge all belong to `person-service`.
