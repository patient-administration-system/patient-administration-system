# 16. Open questions

1. Who is the Clinical Safety Officer? (DTAC-1)
2. How is a patient's identity proven before an account is linked to a `person_id`? Is NHS login the answer in England, and what is the equivalent in Wales and Scotland? (DTAC-9)
3. Which service issues the PASETO tokens in a real deployment, and how does the UI's server obtain its service token (`PAS_API_TOKEN_FILE`)? The API side is done (DTAC-4); the shared token is now only a fallback.
4. When to move to `loco-rs` 1.x, which needs `sea-orm` 2 across the crate. (DEP-2)
5. What is the written consent basis for each message channel, before WhatsApp or Apple Messages are switched on? (NOT-5, DTAC-2)
6. Should a template change need a second person's approval before it goes live?
7. Which carrier per channel, given NHS-approved supplier constraints?
8. Who reviews the machine-translated catalogues, in particular Welsh? (REF-5)
