# Lessons: the Tellescope API reference

Researched 2026-11-17 from the whole of <https://docs.tellescope.com/api-documentation>: one page of about 7.9 MB of HTML, read as text section by section (94 sections: *Getting Started* and 93 models, `AiConversations` to `Webhooks`). The reference documents endpoints, constraints and access rules; most field types are given only as `string` or `number`, so the **values of enums (event, action and trigger types) are not published** there. Nothing was called, and no account was used. The product-level comparison is in [`../tellescope/index.md`](../tellescope/index.md); ideas are in [`plan.md`](../../../plan.md) §21 and `tasks.md` (series `TSC`, TSC-9 to TSC-13, and amendments to TSC-1, TSC-2, TSC-4 and TSC-5).

## What the reference shows

**Conventions.** One REST shape for every model: create one or many (a `create` array), get one, get a page (`limit`, `lastId`, `sort` of `oldFirst` or `newFirst`, `search.query`, `filter`, `from` and `to` on creation time, `fromUpdated` for incremental sync), update (`updates`; arrays append and objects merge unless `replaceObjectFields`), delete. Staff are *users*, patients and other contacts are *endusers*, each with their own sessions; public endpoints need no authentication. Uniqueness and access rules are listed per model as **constraints** ("title must be unique", "when created by an enduser, enduserId must match their id", "only admin users can update user roles").

**The model, by area** (93 models):

| Area | Models |
|---|---|
| People and access | `Users`, `Endusers`, `EnduserCustomTypes`, `RoleBasedAccessPermissions`, `ApiKeys`, `Organizations`, `UserLogs`, `UserNotifications` |
| Automation | `Journeys` (named states with a default), `AutomationSteps`, `AutomationTriggers`, `AutomatedActions`, `FlowchartNotes`, `Waitlists` (a count released into a journey) |
| Staff work | `Tickets`, `TicketQueues`, `TicketTemplates`, `EnduserTasks`, `TimeTracks`, `Notes` |
| Scheduling | `CalendarEvents`, `CalendarEventTemplates`, `CalendarEventRSVPs`, `AvailabilityBlocks`, `AppointmentBookingPages`, `AppointmentLocations`, `Meetings` |
| Forms | `Forms`, `FormFields`, `FormGroups`, `FormResponses` |
| Messaging | `SmsMessages`, `Emails`, `ChatRooms`, `ChatMessages`, `GroupMmsConversations`, `PhoneCalls`, `PhoneTrees`, `CallHoldQueues`, `InboxThreads`, `Templates`, `MessageTemplateSnippets`, `BlockedPhones`, `EmailSyncDenials`, `FaxLogs` |
| Views and reports | `AnalyticsFrames`, `CustomDashboards`, `EnduserViews`, `EnduserProfileViews`, `TableViews` |
| Integration and operations | `Webhooks`, `WebhookLogs`, `Integrations`, `IntegrationLogs`, `BackgroundErrors`, `Configurations`, `Files`, `Databases`, `DatabaseRecords` |
| AI | `AiConversations`, `AiDocuments`, `AiDocumentVersions` |
| Clinical and commercial (out of scope here) | `CarePlans`, `EnduserObservations`, `EnduserOrders`, `EnduserMedications`, `EnduserProblems`, `EnduserEncounters`, `EnduserEligibilityResults`, `AllergyCodes`, `DiagnosisCodes`, `VitalConfigurations`, `PrescriptionRoutes`, `Products`, `Purchases`, `PurchaseCredits`, `OrganizationPayments`, `Superbills`, `SuperbillProviders`, `Forums`, `ForumPosts`, `PostComments`, `PostLikes`, `CommentLikes`, `ManagedContentRecords`, `ManagedContentRecordAssignments`, `PortalBrandings`, `PortalCustomizations`, `SuggestedContacts`, `EngagementEvents`, `EnduserStatusUpdates`, `TicketThreads` and `TicketThreadComments` (Zendesk) |

**Design details worth noting.**

- **Separation of duties, as constraints.** *"Users cannot review their own time tracks."* A locked time record accepts only review fields, or the owner resubmitting a rejected one; a correction keeps the original duration beside the new one.
- **Monotonic revocation.** `invalidateSessionsBefore` *"can only be set forward in time, never backwards"*, for users and endusers alike.
- **Minimised staff absence.** *Out of Office Status* returns *"only user ids and absolute time windows — never event titles or descriptions"*, visible to peers so work can be covered.
- **Work queues are pulled.** *Assign From Queue* "takes a specific ticket (or next available) from a queue and assigns to the caller"; closing a ticket returns the tickets that follow from it in the journey; bulk assignment can add the new owner to the patient's care team.
- **A reply ends a sequence.** *Handle Incoming Communication* "handles removing endusers from relevant journeys" when they write in.
- **Unsubscribe by kind.** *Unsubscribe Enduser* removes a patient from "one or more types of notifications", not only a channel; sending an SMS or email is refused unless the patient has the address *and* consent.
- **Forms save as they go.** *Save Field Response* stores one answer of a partial response and logs a field view; a form carries a display-text translation map per language; a form group issues several forms as one.
- **Echo detection.** Webhook delivery to your own API key's changes is avoided with `dontSendWebhook`, and *"we rate limit requests which perform the same update to the same record to help you detect echo during development"*.
- **Restore from the audit log.** *Restore a deleted record* re-creates a deleted contact and what was cascade-deleted with it from the audit log, idempotently and best-effort.
- **Versioned definitions.** AI apps and agents are a mutable head over immutable numbered versions; a save with a stale `expectedVersion` gets `409`; only an admin may set who it runs as; approvers, a credit limit and a schedule are fields of the definition.
- **Scoped keys, shown once.** An API key's secret is returned once and not stored; scopes (for example `form-ingestion`) restrict a key to matching endpoints and need an admin to set.
- **Operator-visible failures.** `BackgroundErrors` (mark read), `IntegrationLogs` (read only) and `WebhookLogs` give an operator a view of what failed in the background.
- **Out-of-office auto-reply, throttled** "to one message per hour".

## Against this repository

| Topic | Tellescope API | This repository |
|---|---|---|
| Reviewing one's own work | Refused for time records | Refused for session template versions (`approved_by <> proposed_by`, a database check). Follow-up policies, virtual-visit services, WhatsApp template approvals and the no-show reviews record one approver with no stated rule on whether the approver may be the one who asked |
| Revoking sessions | `invalidateSessionsBefore`, forward only | Short-lived PASETO tokens verified offline; revocation belongs to the authentication-service |
| Staff absence | Out-of-office windows, ids and times only | A slot can be blocked (`busy-unavailable` in FHIR). Nothing found that lists the **booked** appointments a new absence affects *(text search)* |
| Unsubscribing | By notification type, and consent per channel | By channel only (`sms`, `whatsapp`, `apple_messages`, `email`) |
| A reply stops prompts | Removes the patient from journeys | Per flow (TSC-2) |
| Work queue | Pull "next available", follow-on tickets on close | Separate lists (TSC-1) |
| Partial forms, translations | Saved per field; translation map | No forms (TSC-4); every patient-facing text is translated in `locales/` |
| Webhook integrity | `sha256(ids + timestamp + secret)`: a plain hash over the record ids, not the body, but with a timestamp | HMAC-SHA256 over the whole body (`X-PAS-Signature`, `streaming/webhook.rs`), stronger, but **with no timestamp**, so a captured delivery can be replayed and the receiver cannot tell |
| Echo | `dontSendWebhook`, and a rate limit on repeated identical updates | Neither. An integration that writes back on our webhook could loop unseen |
| Restoring a deletion | From the audit log | The PAS cancels, closes and withdraws rather than deleting rows that matter |
| Versioned configuration | Immutable versions, `409` on a stale save | Session templates are versioned with approval; letters store the rendered text that was sent, so what a patient received is kept |
| Background failures | Errors, integration and webhook logs | Dead letters with replay (HUB-4), health that reports a stalled outbox or listener (HUB-6) |

## Lessons worth taking

1. **Who may approve what, written down and enforced.** Make an inventory of every approval in the PAS and decide for each whether the approver may be the person who proposed or requested it; where not, refuse it in the database, as session templates already do. → TSC-9.
2. **Staff absence makes work, not cancellations.** Record a clinician's or room's unavailability as a time window only (no reason, no detail), list the booked appointments it touches, and put each in the work queue for a person to rebook with the patient. Nothing is cancelled automatically. → TSC-10.
3. **Opting out of a kind of message.** A patient who wants no surveys or experiments should keep their appointment messages. Service messages (an appointment's booking, change and reminder) stay governed by the channel opt-out; the optional kinds get their own. → TSC-11.
4. **Sign the time with the body.** Put a timestamp in the signed text (`t=<unix seconds>`, then the body) and send it in the signature header, so a receiver can refuse a delivery older than a few minutes and a captured one cannot be replayed; keep the body HMAC, and the event id for de-duplication. → TSC-13.
5. **Catch an integration that loops.** When the same credential makes the same change to the same record again and again in a short time, refuse it with `429` and log it once, so a write-back loop is seen in development, not in production. → TSC-12.
6. **Into existing tasks:** a queue that is pulled ("take the next") and items that create their follow-on when closed (TSC-1); a reply from the patient stops the prompts that asked for it (TSC-2); partial answers saved per field and a translation per language (TSC-4); a forward-only "not before" for revoking a person's or an integration's tokens, with the authentication-service (TSC-5).

## Not taken, and why

- **`dontSendWebhook`** and *bulk assign without webhooks*: a change no subscriber hears of is a silent change.
- **`relatedRecords` in webhooks** as a cache that "may not include all related ids": a receiver that trusts it acts on stale data.
- **Webhook integrity as a plain hash of ids:** weaker than the HMAC over the body we already use; only its timestamp is taken (TSC-13).
- **Arrays that append and objects that merge on update by default**: surprising, and hard to audit.
- **Custom MongoDB aggregations and proxied reads and writes to integrations over the API**: unaudited reads and a wide attack surface.
- **Restoring deletions from the audit log**: the PAS does not hard-delete the rows that matter.
- **Patient logins, passwords and OTPs; forums, products, purchases, superbills, prescriptions, labs, vitals, fax, phone trees and hold queues**: identity belongs to person-service; the rest is clinical, commercial or telephony, out of scope.
- **AI agents**: none planned. If one ever is, Tellescope's governance fields (immutable versions, approvers, a spending cap, who it runs as set only by an admin) are the minimum, and a hazard assessment comes first.

## Limits of this research

The reference publishes endpoints and constraints, not behaviour: enum values are mostly absent, so the full lists of triggers, events and actions are not known. Nothing was run. "Nothing found" statements about this repository are text searches of `src/`, `domain/src/` and the migrations.
