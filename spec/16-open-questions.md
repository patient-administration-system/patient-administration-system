# 16. Open questions

1. Who is the Clinical Safety Officer? (DTAC-1)
2. How is a patient's identity proven before an account is linked to a `person_id`? Is NHS login the answer in England, and what is the equivalent in Wales and Scotland? (DTAC-9)
3. Do pilots need per-person tokens at the API (a full authentication service), or is the UI's server acting for people via `pas-token-issuer` and `X-On-Behalf-Of` enough? (DTAC-4)
4. When to move to `loco-rs` 1.x, which needs `sea-orm` 2 across the crate. (DEP-2)
5. What is the written consent basis for each message channel, before WhatsApp or Apple Messages are switched on? (NOT-5, DTAC-2)
6. Should a template change need a second person's approval before it goes live?
7. Which carrier per channel, given NHS-approved supplier constraints?
8. Who reviews the machine-translated catalogues, in particular Welsh? (REF-5)
9. Which deprivation index, which postcode file and which licence will the health board use to fill the equity bands, and who builds the file? (`governance/deprivation-data-route.md`)
10. Is the no-show score a medical device, and who is the named fairness reviewer and clinical safety reviewer? Which clinic goes live first, and why that one? (`governance/noshow-model.md`, `governance/noshow-evaluation.md`)
11. What are the current Welsh Ambulance Service NEPTS criteria and booking route, and who may request transport for a patient? (`governance/nepts-criteria.md`)
12. Who may use the analytics feed, under what data-sharing terms, and should the open feed be replaced by a fixed monthly publication? (`governance/analytics-differencing.md`)
13. Should a failed reminder send be retried within its step's window, rather than left to the next step and the call to a person? (`governance/chaos-testing.md`)
14. Who sets the tolerances (target, tolerance, limit) for waits, lead time, missed appointments, slot use, bed occupancy and money collected, and should they differ by service or ward? The built-in defaults are starting points only. (TOL-1)
