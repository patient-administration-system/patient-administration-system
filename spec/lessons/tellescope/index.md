# Lessons: Tellescope

Researched 2026-11-17 from <https://www.tellescope.com/features> and, from there, the feature pages for workflow automation, patient intake, appointment scheduling, the communication platform, patient relationship management and EHR integration; the use-case pages for care coordination, remote patient monitoring and patient tracking; and part of the API reference at <https://docs.tellescope.com/api-documentation> (about the first 200,000 of its 279,000 characters: AI conversations and documents, analytics frames, API keys, booking pages, locations and automated actions). The Trust Center (<https://trust.tellescope.com/>) rendered no text, so **no compliance claim beyond the word "HIPAA" was read**. Nothing was run, and no account was used. Ideas are in [`plan.md`](../../../plan.md) §20 and `tasks.md` (series `TSC`).

## What it is

A United States, commercial, closed-source patient engagement platform: a CRM of patients (*endusers*) with a low-code **journey** engine, forms, booking pages, two-way SMS, email, phone and video, a team inbox, tasks (*tickets*), analytics and an API "first" design, sold to digital-health companies and clinics, often as a white-labelled front end beside an EHR (Canvas and Healthie are named in the API). Its market is US virtual care: state licensure, insurance cards, Stripe and Chargebee subscriptions, remote patient monitoring billing, prescriptions.

The marketing pages are short and general: features are one-line claims, and the outcome figures ("85% reduction in manual tasks", "25% increase in patient engagement") have no stated source or method. The API reference is the useful part, because it shows how the product actually works.

## How it works (from the API reference)

- **A journey is a graph of automation steps.** Each step waits for an **event** (a form submitted, an appointment created or changed, a field changed, a tag added, another step's outcome), optionally after a **delay** that can be limited to **office hours**, computed in the **patient's time zone**, and **skipped if it has already passed**; it may require a **condition** (on a form answer, a profile field, an event, or the day of the month); then it runs an **action**.
- **Actions:** send an email, SMS or form; add a tag; set or increment a field; enrol in another journey; call a webhook; ask an AI to choose an outcome or place a call; create a **ticket**; billing and prescription actions.
- **Cancel conditions.** A scheduled action carries the conditions that cancel it, with the reason and who cancelled it recorded: a reminder to complete a form is cancelled when the form arrives.
- **Tickets are the staff work queue.** A ticket has a title, a priority, an **assignment strategy**, a queue, a **due date as an offset that can skip days of the week**, **reminders before it is due**, an optional confirmation, and follow-on actions.
- **Booking pages** list appointment templates and locations and restrict who may book by care team, US state, tags, and **how many hours ahead** booking is allowed; one-time access tokens expire.
- **Analytics frames** are saved report definitions over most models, with filters, grouping and date ranges, and visibility by role and user.
- **API conventions:** per-model create, update and delete webhooks (an update can say `dontSendWebhook`); `fromUpdated` for incremental sync; `lastId` pagination; **scoped API keys** (for example a key that may only ingest forms); a 409 when a saved version is stale.

## Against this repository

| Topic | Tellescope | This repository |
|---|---|---|
| Patient record | Its own CRM of endusers, tags, custom fields, stages | Identity is owned by person-service; the PAS keeps references (`person_id`) and its own administrative rows. No tags or custom fields, by design |
| Patient timeline | One timeline across channels | `GET /api/patients/{id}/contact-log` |
| Automation | A no-code journey builder anyone with access can change | Fixed, reviewed flows in code: the reminder ladder (`communication/ladder.rs`), follow-up, PIFU, standby offers, rebooking offers, reply links, write-back, list validation; each with tests and a hazard row where it matters |
| Late steps | `skipIfDelayPassed` | The ladder runs only the latest of several due steps and logs the rest as `skipped_late` |
| Quiet hours | `officeHoursOnly`, patient time zone | `PAS_QUIET_HOURS` (`communication/consent.rs`); one service time zone (`PAS_TIMEZONE`) |
| Cancel conditions | Explicit on every scheduled action, with a reason | Per flow, in its own code; no common rule or test that every scheduled send re-checks its purpose before it goes |
| Staff work queue | Tickets: owner, queue, priority, due date in chosen days, reminders | **Separate lists per feature**: `follow_up_tasks`, `staff_call_tasks`, `noshow_reviews`, the communication-needs review list (`needs_review`), list validation, suspicion review, tolerance alerts. No common owner, priority, due date or "mine" view |
| Working days | Due dates skip chosen weekdays | Weekday handling inside single features; no shared working-day or holiday calendar |
| Forms and intake | Form builder, consent, insurance card upload, review before it is kept | **None.** Patients answer only through reply links and inbound SMS keywords |
| Self-booking | Open booking pages with restrictions | Offers only: two or three real slots for an existing appointment, minimum notice, horizon, single use, expiry (`rebooking/mod.rs`, CAP-5); PIFU for patient-initiated follow-up |
| Waiting for an earlier slot | Not described | Standby lists and slot releases (`standby/`) |
| Messaging channels | SMS, email, phone, video, chat, portal | SMS, email, letter, WhatsApp, IVR; virtual visit links (`virtual_visits/`) |
| Webhooks | Per model, create/update/delete | Transactional outbox, HMAC-signed webhooks (no timestamp in the signature: TSC-13), accepted, rejected and deferred classes, dead letters with replay (HUB-4) |
| Incremental sync | `fromUpdated` | **None** on the REST lists, and FHIR search has no `_lastUpdated` |
| Credentials for integrations | Scoped API keys | Per-user PASETO tokens with ABAC; a service token (`svc=true`) **may do anything** |
| Saved reports | Analytics frames with role visibility | Fixed reports (`reports/`, `analytics/`); nothing saved per user or team |
| AI | Journey decisions, AI phone calls, AI summaries, an AI journey builder | None, and none planned in the flow of a patient's care (NFR-11 and the hazard log) |
| Clinical | Remote monitoring thresholds and billing, medications, allergies, prescriptions | Out of scope: the PAS is non-clinical |

## Lessons worth taking

1. **One staff work queue, not one list per feature.** The PAS already creates the right work; it scatters it. A person on a booking team has to know which of seven lists to look at, nothing shows what is overdue, and nothing says who has picked an item up. Tellescope's ticket is the shape: an item linked to the patient and the thing it is about, a queue (team), an owner or none, a priority, a due date, reminders before it is due, and a closing outcome. The existing lists stay as they are and feed it. Support only: an item asks a person to act; it never acts on a patient (NFR-11). → TSC-1.
2. **Cancel conditions as a rule, with a test per flow.** Every scheduled outbound step should re-check, at the moment it would go, that its purpose still holds (the appointment is still booked, the reply has not come, the form has not been returned), and record why it did not go. The ladder already skips late steps; the rule should be stated once and proven for every flow that schedules a send. → TSC-2.
3. **Working-day arithmetic in one place.** "Due in two working days" and "remind three working days before" need weekends and public holidays, and the holidays are national and regional: a configured calendar, not a list in code (NFR-26, QUAL-40). It belongs in the domain crate (QUAL-52). → TSC-3.
4. **An administrative pre-appointment questionnaire.** Not clinical intake: confirm contact details, communication needs (the Accessible Information Standard), an interpreter, transport, reasonable adjustments, and "do you still need this appointment?" (which list validation asks by other means). Sent through the existing reply links; the answers are proposed changes that a person approves, never written straight into the record (the way write-back already works). FHIR `Questionnaire` and `QuestionnaireResponse` give the shape. → TSC-4.
5. **Least privilege for integrations.** A service token that may do anything is the wrong default once there are several integrations (the UI's backend, an HL7 feed, a reporting extract). A scope per integration, checked by the existing ABAC policy, limits what a leaked token can do. The claims come from the authentication-service, so this needs its owner. → TSC-5.
6. **Incremental sync.** An integrator who cannot subscribe to webhooks should be able to ask "what changed since T?" with a stable cursor, on the main REST lists and as FHIR `_lastUpdated`. → TSC-6.
7. **Saved views of the work queue.** Named filters ("my team's overdue", "unassigned urgent") shared by role, after TSC-1. → TSC-7.
8. **Contact that went unanswered, as work.** Tellescope alerts when patients "are disengaged". The PAS equivalent is factual, not a score: every route to the patient was tried and nothing came back (the ladder exhausted, a letter returned, list validation unanswered). That is an item for a person in the work queue. → TSC-8.

## Not taken, and why

- **A no-code journey builder.** It lets anyone with access change what happens to patients, and the hazard log (DCB0129) cannot assess a flow nobody has written yet. The PAS keeps each flow in reviewed code with a test and a hazard row. What transfers is the vocabulary (event, delay, condition, action, cancel condition), used to describe and test the flows we have.
- **AI decisions, AI phone calls, AI summaries.** An automated decision in a patient's care needs a person (UK GDPR Art. 22) and a hazard assessment; none is planned (NFR-11).
- **Remote monitoring, medications, allergies, prescriptions, care plans.** Clinical; out of scope.
- **Open booking pages for new patients.** NHS outpatient care is referral-led; the PAS offers real slots for an appointment that exists, and PIFU for patient-initiated follow-up. Restricting by US state licensure has no UK analogue that the schedules do not already encode.
- **Tags and custom fields on patients.** Identity belongs to person-service, and free tags on a patient drift into unaudited clinical labels.
- **Payments (Stripe, Chargebee) and insurance cards.** NHS care is free at the point of use; private billing is the existing `billing` module.
- **`dontSendWebhook`.** A change that tells no subscriber is a silent change; a backfill should emit its events or be marked as one.
- **Arbitrary database aggregations over the API.** A data-exposure risk with no audit of what was read.
- **Array fields that append by default on update.** Surprising semantics; the PAS replaces and uses `If-Match` (already has the 409 equivalent).
- **White-label theming.** A deployer's concern, met by the Lily Design System tokens.

## Already ours

A patient contact log; quiet hours; late steps skipped; offer-based rebooking with minimum notice, horizon, single use and expiry; standby lists for earlier slots; SMS, email, letter, WhatsApp and IVR with consent and opt-out; signed webhooks through an outbox with failure classes and dead letters; FHIR R5; optimistic concurrency with `If-Match`; cursor pagination; per-user audit.

## Limits of this research

The marketing pages are vague, sometimes inconsistent (headings that do not match their text; "bi-directional" sync whose items are one-way pushes), and their figures are unsourced. This page was written from about two thirds of the API reference; the whole reference was read afterwards, in [`../tellescope-api-documentation/index.md`](../tellescope-api-documentation/index.md). The Trust Center was not readable. "None" and "not found" statements about this repository are text searches of `src/`, `domain/src/` and the migrations.
