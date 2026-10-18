# Match / Search / Merge

> **v0.44.0 Option C: fully delegated.** Person matching, search, dedup, and merge all live in `person-service`. Worker matching, search, dedup, and merge all live in `worker-service`. The PAS calls those services over HTTP and proxies the result through its legacy `/api/patients/*` and `/api/practitioners/*` URLs. The PAS no longer holds any matching code, search index, or merge tombstone logic — every section below is **v0.43.0 archaeology** until Layer 11C lands.

> **Shipped v0.43.0 (below).** Probabilistic *decisions* about identity belong to the sister MPI crate (`master-patient-index-rust-crate`, now realized as `person-service`). The PAS implements the deterministic pieces — search, exact-MRN dedup on HL7 v2 ingest, and the merge *application* — and leaves the decisioning to MPI / operators.

## In scope for the PAS

### Search

Tantivy-backed full-text search over the patient demographic snapshot.

- Schema fields: `id`, `family_name`, `given_names`, `birth_date`, `mrn`, `postal_code`.
- `SearchEngine::new(path)` opens or creates the index at the configured `SEARCH_INDEX_PATH`.
- `index_patient`, `delete_patient`, and `search(query, opts)` keep the index in lockstep with patient create / update / delete / merge handlers.
- Exposed via `GET /api/patients/search?q=…&limit=N`.

### Match — exact-MRN dedup on HL7 v2 ADT^A01 / ^A28

PID-3.1 lookup via Postgres JSONB `@>` containment query. If a non-deleted patient with that MRN exists *and* the matched row actually carries that MRN type, the existing patient is reused — no duplicate insert, no audit, no Tantivy re-index. AA ACK reports `matched existing patient <uuid>` in MSA-3. This is the PAS's only matching surface; everything probabilistic is the MPI's job.

### Merge tombstone (v0.11)

When the MPI determines two PAS patient rows refer to the same person (or an operator decides), call **`POST /api/patients/{source_id}/merge-into/{target_id}`**:

- The source row becomes a **merge tombstone**: `replaced_by = target_id`, `active = false`. Both rows survive in the database for audit; the source is dropped from the Tantivy search index and excluded from default `GET /api/patients` listings.
- FHIR R5 surfaces the link as `Patient.link[type = "replaced-by"]` on the tombstone.
- An `audit_log` row (`action = "merge_into"`) and an outbox `PatientMerged { source_id, target_id }` event fire in the same transaction.
- The inverse direction is available at `GET /api/patients/{id}/replaces` (every tombstone pointing at this row, newest-first).
- **HL7 v2 ADT^A40** (v0.18) is the same operation arriving over HL7 v2: survivor in PID, source MRN in MRG-1.1. Outbound `Hl7v2MllpPublisher` emits ADT^A40 on `PatientMerged` events, source-gated to avoid echoing inbound A40 back to the EMR that sent it.

Conflict matrix: self-merge (`id == target_id`) → 400; source already a tombstone → 409 (no chained merges); either row missing → 404.

### Identity linkage with MPI

- `Patient.mpi_id: Option<Uuid>` is the only link to the MPI identity service. The PAS does not depend on the MPI crate; identity resolution is out-of-band.
- Merge-back integration: the MPI calls the merge endpoint above; the PAS does not call the MPI.

## Out of scope for the PAS

These features live in the MPI crate, not here:

- Probabilistic matching (Jaro-Winkler / weighted field-by-field scoring with confidence thresholds).
- Soundex / phonetic matching.
- Real-time fuzzy duplicate detection on registration (returning 409 from a similarity score).
- Batch fuzzy duplicate detection with a review queue.
- Merge *decisioning* (picking the survivor; combining demographics; resolving conflicts).
- Geo-radius search.

If a deployment needs probabilistic matching it integrates with the MPI as a separate service. `Patient.mpi_id` is the integration seam, and the merge endpoint above is how the MPI's decisions land in the PAS.
