# Decision: the PAS validates NHS numbers and does not call PDS

**Status:** decided 2026-10-19 (DTAC-8). Review when person-service plans a PDS trace, or if the PAS ever
needs to *find* a patient by NHS number rather than receive one.

## What the PAS does

- `POST /api/validate/nhs-number` checks that a value is ten digits and that its last digit is the correct
  modulus-11 check digit of the first nine. It stores nothing.
- Creating or updating a patient through `/api/patients` refuses a body whose `identifiers` include an NHS
  number (system `https://fhir.nhs.uk/Id/nhs-number`) that fails the check, before it reaches person-service.
- Nothing else about an NHS number is checked: not that it was ever issued, not that it belongs to the
  person it is attached to, not that the person is alive.

## What the PAS does not do, and why

A look-up on the Personal Demographics Service (PDS) answers those questions. The PAS does not make one:

1. **It is not the PAS's data.** Patient demographics, including identifiers, are held by person-service
   (`spec/09`, "Not here"). A PDS trace belongs next to the record it would correct.
2. **It needs onboarding the PAS has not done.** PDS FHIR access needs an application registered with NHS
   England, an ODS code for the calling organisation, signed-in or application-restricted credentials, and a
   clinical-safety case for what happens when a trace is wrong or unavailable. None of that exists yet.
3. **A wrong trace is a patient-safety risk**, not a convenience (DCB0129, DTAC-1). It should be designed
   with the Clinical Safety Officer, not bolted on.

## Consequence to accept

A well-formed NHS number can still be someone else's, or unissued. Staff must not treat "valid" as "verified".
The endpoint's own wording says so ("well formed"), and so does its documentation.

## When to revisit

- person-service adds a PDS trace: the PAS should call it, not PDS.
- The PAS must match an inbound referral or reply to a patient by NHS number.
- A pilot site requires PDS verification as a condition of go-live.
