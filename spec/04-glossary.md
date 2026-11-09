# 4. Glossary

| Term | Meaning |
|---|---|
| ADT | Admit, discharge, transfer: the events that move a patient through a hospital |
| BFF | Backend-for-frontend: the SvelteKit server that holds tokens so the browser never does |
| DNA | Did not attend: a missed appointment |
| DTAC | NHS Digital Technology Assessment Criteria |
| ODP | Operating Department Practitioner: works across anaesthetics, surgery and recovery in the operating theatre |
| Outbox | Table of events written in the same transaction as the change, forwarded afterwards |
| PAS | Patient Administration System |
| Portal | The signed-in area where a patient reads messages (`/my/messages`) |
| `person_id` | The upstream `person-service` id of a patient |
| RTT | Referral-to-treatment: the clock measuring a wait for care |
| Point of suspicion | The date a clinician first suspected a condition; the start of a suspected condition's clock (spec 20) |
| Suspected condition | A condition a referral is made on suspicion of, before a diagnosis: cancer, sleep apnoea, diabetes, dementia and others a profile configures (spec 20) |
| Suspicion profile | The configuration of which conditions are tracked and each one's subtypes, target, cancel rule and booking priority (`PAS_SUSPICION_PROFILE`, spec 20) |
| Clock stop | The one date that ends a suspected condition's wait: first definitive treatment, a downgrade (ruled out), or death |
| Downgrade | A suspicion of cancer ruled out; it stops the clock |
| SCP | Suspected Cancer Pathway (formerly the Single Cancer Pathway) in Wales: 62 days from the point of suspicion to first definitive treatment |
| Tumour site | The primary cancer site of a cancer suspicion: a subtype, from the list the profile names (the Welsh "Primary Cancer Site Description" in the `wales` profile) |
| `worker_id` | The upstream `worker-service` id of a staff member |
