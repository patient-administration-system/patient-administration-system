# Record of processing (DRAFT, UK GDPR Article 30)

The controller is the deploying organisation, not the maintainers. This is a starting point for the controller to complete.

| Activity | Purpose | Data subjects | Categories | Recipients | Retention class |
|---|---|---|---|---|---|
| Admission, transfer, discharge | Manage inpatient care administration | Patients | `person_id`, dates, bed, reason, disposition | Clinical systems (HL7 v2) | record |
| Appointments and scheduling | Book and manage outpatient care | Patients | `person_id`, time, status, reason, cancellation reason | Clinical systems, the patient | record |
| Waiting list and RTT | Meet waiting-time standards | Patients | `person_id`, service, priority, clock events | Commissioners (aggregated) | record |
| Reminders and letters | Reduce missed appointments; inform patients | Patients | Rendered text, channel, status | Message carriers (when built) | message |
| Consent records | Record lawful basis for optional processing | Patients | `person_id`, type, dates | None | record |
| Billing and coverage | Charge and reconcile | Patients, subscribers | Payor, policy, amounts | Payors | billing |
| Audit | Accountability and safety | Patients, staff | Old/new values, user id, IP, user agent | Auditors on request | audit |
| Patient portal | Let patients read their own appointments and messages | Patients | Session, `person_id` claim | Identity provider | session |

Lawful bases and special-category conditions are for the controller to state. They are not decided here.
