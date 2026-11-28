# Deviations from the standards (HUB-10)

**Status: started 2026-11-17 from what the spec already says in passing; the Decision column is for the
maintainer (or a named owner) to fill in, not the author.** Each row is a place where the PAS knowingly does
something other than what HL7 v2, FHIR or an IHE profile describes, or chooses one reading where the standard
leaves a choice. A golden test may be re-captured only with a row here, or an FR or NFR, named in the commit
(`tests/hub7_hl7_golden_test.rs`).

Kinds: **gap** (part of the standard not built, refused rather than ignored), **choice** (the standard allows
more than one way; this is ours), **departure** (the PAS does something the standard does not describe).

| ID | Standard | What the standard says or allows | What the PAS does | Kind | Source | Decision |
|---|---|---|---|---|---|---|
| DEV-1 | HL7 v2.5, chapter 2, acknowledgement modes | Original mode (an application ACK per message) or enhanced mode (a commit ACK, then an application ACK, chosen by MSH-15 and MSH-16) | Original mode only. MSH-15 and MSH-16 are not read; no commit ACK (`CA`, `CE`, `CR`) is ever sent | gap | `src/hl7v2/ack.rs`; no code reads MSH-15 or MSH-16 | |
| DEV-2 | HL7 v2.5, chapter 2, MSA and ERR | An error ACK should carry an `ERR` segment with a coded reason (table 0357, such as `200` unsupported message type); MSA-3 text is kept for compatibility | The reason is MSA-3 text only, one line of at most 80 characters, naming a field and a reason and never a value. No `ERR` segment | departure | `hl7v2::ack::ack_to`, NFR-31, NFR-33 | |
| DEV-3 | HL7 v2.5, chapter 2, duplicate messages | Duplicate detection is left to the receiver (sequence numbers in MSH-13 in enhanced mode) | A message whose MSH-3, MSH-4 and MSH-10 were already processed is answered from its first ACK and not processed again, for `PAS_HL7_DEDUPE_DAYS` (7); a copy that did not finish is refused with an AE | choice | `hl7v2::dedupe`, NFR-31, HUB-2 | |
| DEV-4 | HL7 v2.5, chapter 2, message type | The receiver reads MSH-9 to know what it was sent | Over MLLP and in a batch, MSH-9 picks the handler and a type the PAS does not handle is answered AR. Over HTTP the route names the type, and since 2026-11-28 each mutating path takes only the MSH-9 type the same table gives it: an `ORU^R01` or an `ADT^A03` posted to `/api/hl7/v2/admit` is answered AR (`MSH-9 does not match this path, which takes ADT^A01`), changes nothing and is not remembered, so the same message sent to the right path is processed | choice | `hl7v2::routing` (`ROUTES`, `refusal`), `api::rest::hl7_dedupe`, `tests/dev4_hl7_message_type_test.rs`, `tests/golden/hl7/hl7-type-check.json` | 2026-11-28, the owner (audit PA-16): each path takes exactly the type the routing table gives it, nothing extra; a sender that posts another type must be corrected, not accommodated |
| DEV-5 | HL7 v2.5, chapter 2, batch acknowledgement | A file (`FHS`/`FTS`) or batch (`BHS`/`BTS`) may be acknowledged as a whole or per message | Per message, inside one `BHS`/`BTS` envelope; a file header (`FHS`) is answered with a batch envelope, not a file one | choice | `hl7v2::batch::encode_batch_ack` | |
| DEV-6 | IHE ITI Scheduling 1.0.0 | The profile is FHIR R4 | The PAS's FHIR endpoint is R5; the profile's operations are under `/fhir/r4` with their own CapabilityStatement and are not in the R5 one | departure | `spec/09-api-surface.md` (IHE ITI Scheduling) | |
| DEV-7 | IHE ITI Scheduling 1.0.0, ITI-115 to ITI-118 | Modify and cancel through `$book`, a new patient, group appointments, search filters, ITI-118, paging | Not built; each is refused by name, not ignored | gap | `spec/09-api-surface.md` (IHE ITI Scheduling) | |
| DEV-8 | FHIR R5, transaction Bundle | A server may accept any resource type in a transaction | `Patient` and `Practitioner` entries are refused with `400`, because other services own them and cannot share the transaction; other types are `501` | departure | `spec/09-api-surface.md` (FHIR writes) | |
| DEV-9 | HL7 v2 and FHIR writes, concurrency | Neither standard carries a version for the sender to send | They are applied as if `If-Match: *`: under the row lock a screen uses, but a stale view is not refused | choice | `spec/09-api-surface.md` (OFH-10) | |

## How to add a row

Name the standard and the section; say what it says and what the PAS does, without judging; give the code or
spec section that does it; leave Decision empty. A row is never deleted: one that no longer applies says so
in Decision, with the date.
