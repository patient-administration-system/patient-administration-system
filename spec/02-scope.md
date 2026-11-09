# 2. Scope

## In scope

- Admission, transfer and discharge (ADT), with bed and ward occupancy.
- Appointments, slots, schedules and recurring series.
- Waiting lists and referral-to-treatment (RTT) clocks.
- Tracking suspicion during referrals: the condition a referral is made on suspicion of (cancer, sleep apnoea, diabetes, cardiovascular disease, Parkinson's, Alzheimer's), its point of suspicion and the wait until its clock stops ([20](20-suspected-conditions.md)). The PAS records a clinician's suspicion and decisions; it makes none.
- Letters and patient notifications: postal letter, email, SMS, WhatsApp, Apple Messages and the secure portal.
- Appointment reminders with administrator-edited templates.
- Consent records, coverage and billing records.
- An audit log and a transactional event outbox.
- Interchange: REST/JSON with OpenAPI, FHIR R5, HL7 v2.
- A browser UI for patients, staff and administrators.
- Recording a triage category that someone else assigned to an emergency encounter, and measuring the wait against its target (NEW-22, NEW-23).

## Out of scope

- Patient and worker demographics and their master index (`person-service`, `worker-service`).
- Clinical records, orders, results, prescribing.
- Deciding a triage category: the Manchester Triage System flowcharts and discriminators are licensed content and a clinical decision, and are not in this repository (NEW-25).
- Running the identity provider in production (the repository ships a development Keycloak realm only).
- Operating a hosted service. The maintainers publish software, not a service.

## Not yet in scope but planned

Real carrier integrations for each message channel, two-way replies, released-slot backfill, a risk score that only ever adds support. See [`../tasks.md`](../tasks.md).
