# 5. Domain model

Authoritative field-level detail is in the server's [`spec.md`](../patient-administration-system-with-rust/spec.md) and `src/models/`. This chapter fixes the vocabulary and the rules other chapters rely on.

```mermaid
erDiagram
    FACILITY ||--o{ WARD : has
    WARD ||--o{ ROOM : has
    ROOM ||--o{ BED : has
    ENCOUNTER ||--o| ADMISSION : "may become"
    ADMISSION ||--o| DISCHARGE : ends
    SCHEDULE ||--o{ SLOT : offers
    SLOT ||--o| APPOINTMENT : "booked as"
    APPOINTMENT_SERIES ||--o{ APPOINTMENT : generates
    WAITLIST_ENTRY }o--|| RTT_PATHWAY : "clock"
    LETTER_TEMPLATE ||--o{ GENERATED_LETTER : renders
    APPOINTMENT ||--o{ GENERATED_LETTER : "reminded by"
```

People are not modelled here. Rows carry `person_id` (patient) and `worker_id` (staff) from the upstream services.

## Appointment status

`proposed → booked → arrived → fulfilled`, with `cancelled` and `no_show` as terminal states. A transition to the same state, or out of a terminal state, MUST be rejected.

## Planned encounters (PCC-8)

The Care Coordination guide (a draft proof of concept) says future activity is an `Appointment` and a registration never holds a `planned` encounter. We keep one narrow exception: **`pre_admit` creates an inpatient encounter in `Planned`** (HL7 v2 `ADT^A05`, undone by `ADT^A38`, `cancel_pre_admit`). Decided 2026-11-02, by me from the code, not yet by a records owner.

- **Why it stays.** A pre-admission is not a booked slot: it has no schedule and no slot, it holds a bed (`Reserved`), and it has its own message pair. An `Appointment` would have to invent a slot to carry it. `Planned → Arrived` and `Planned → Cancelled` are the states `admit` and `cancel_pre_admit` already use.
- **What is not allowed.** Nothing else creates a `Planned` encounter: an outpatient or clinic visit that is in the future is an `Appointment` and only becomes an encounter when the patient arrives. A new feature that books a future visit MUST use an `Appointment`.
- **Cost.** Anything that counts encounters must count by status, or it counts pre-admissions that never happened. I have not checked the reports for this.
- **Open.** The guide's reason for the rule was not read, only its statement of it (`spec/compare/patient-care-coordination/`). OFH-4 (a hold with an expiry on the bed) would also settle what happens to a `Planned` encounter whose reservation lapses.

## Bed reservations (OFH-4)

A pre-admission (`ADT^A05`) reserves a bed (`Available → Reserved`) and opens a `Planned` inpatient encounter. Since OFH-4 it also records a **reservation**: the bed, the person, the planned encounter, who made it, when, and an optional `expires_at`.

| Reservation | Meaning |
|---|---|
| open (`released_at` empty) | The bed is held. At most one open reservation per bed. |
| `released`, reason `cancelled` | `cancel-pre-admit` (`ADT^A38`) closed it; bed `Available`, encounter `Cancelled`. |
| `released`, reason `expired` | `expires_at` passed; a job gave the bed back and cancelled the planned encounter. |

`expires_at` is set from the request's `hold_minutes`, else `PAS_BED_HOLD_MINUTES`; with neither the hold never lapses, as before. Every change is a row in `bed_occupancy_events` (`reserved`, `cancelled`, `expired`, with the person and the actor). **Not covered:** `admit` still accepts only an `Available` bed, so a reservation is never closed by the admission it was for; the occupancy log holds reservations only, not admissions, transfers or discharges (those are in `bed_assignments`).

## Referral status (PCC-2)

A referral has six states. `received` is where every referral starts; `closed`, `returned_with_advice` and `redirected` are final. The code holds the table as `referrals::is_legal_move`, and the database test `every_move_of_the_lifecycle_table_is_allowed_or_refused_as_written` tries every from-state against every move through the real operations.

| From | Legal moves (operation) | Refused |
|---|---|---|
| `received` | `waitlisted` (waitlist), `booked` (book), `closed` (close), `returned_with_advice` or `redirected` (triage) | back to `received` |
| `waitlisted` | `booked` (book), `closed` (close) | waitlisted again, triage, back to `received` |
| `booked` | `closed` (close) | booked again, waitlisted, triage |
| `closed`, `returned_with_advice`, `redirected` | none | every move |

Rules behind the table:

- Triage happens once and only on a referral just received (NEW-11); a redirect creates the replacement referral in the same transaction, so the patient is never left without one.
- A booked referral can still be closed. That is the one move out of `booked`, and it is not the same thing as a cancelled appointment: cancelling the appointment does not change the referral's status today.
- **The guide's edge rules we do not have** (a decline after acceptance, a cancel request that loses to an outcome that arrived first, a no-show that rebooks or ends the referral, a deferred referral that resumes). They need states we do not hold, and a no-show or a cancelled appointment does not touch the referral today. Whether a service wants them is PCC-3, and it is asked of a referral-handling service, not decided from a draft guide.
- A booked referral can still be closed. That is the one move out of `booked`.

The guide (PatientCareCoordination, a draft proof of concept) has twelve states: `requested`, `accepted`, `declined`, `information_requested`, `redirected`, `deferred`, `appointment_booked`, `no_show`, `interim_report`, `cancel_requested`, `cancelled`, `completed`. Mapping, as I read it and not checked against IHE 360X:

| Guide state | Here |
|---|---|
| `requested` | `received` |
| `appointment_booked` | `booked` (and `waitlisted` before it, which the guide has no state for) |
| `redirected` | `redirected` |
| `completed`, `cancelled` | `closed` (one state; the reason is in the audit log) |
| `declined`, `information_requested` | nearest is `returned_with_advice` |
| `accepted`, `deferred`, `no_show`, `interim_report`, `cancel_requested` | none |

We receive and triage referrals; we do not tell the sender (PCC-4).

## Delivery channels

`print` (postal letter), `email`, `sms`, `whatsapp`, `apple_messages`, `portal`. A template lists its channels in preference order.

| Channel | Recipient | Delivered when |
|---|---|---|
| `print` | none | an operator marks the letter sent |
| `email` | patient's email | the email sender accepts it |
| `sms`, `whatsapp` | patient's phone | the sender accepts it |
| `apple_messages` | phone, else email | the sender accepts it |
| `portal` | none (the patient signs in) | the moment it is stored |

A letter has status `pending`, `sent` or `failed`. A `sent` or `pending` letter for an appointment and template means that appointment has been reminded.

## Template variables

`patient` (the upstream person), `appointment` (`id`, `start`, `end`, `reason`) for reminders, and `portal_url` when `PAS_PORTAL_URL` is set.
