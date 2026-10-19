# Patient transport: the NEPTS criteria (to be verified before relying on them)

Transport (CAP-9) is built behind a provider interface (`TransportClient`) with a logging stub; there is **no real provider connected**. In Wales, non-emergency patient transport (NEPTS) is run by the Welsh Ambulance Service under criteria set by Welsh Government, and they have their own eligibility test. The PAS must not make or imply a promise that transport is available.

## What the public pages say (read 2026-10-19; verify)

- NEPTS is for people who, for **medical reasons**, cannot make their own way to and from healthcare appointments ([Welsh Ambulance: NEPTS criteria](https://ambulance.nhs.wales/services/non-emergency-patient-transport-service-nepts/nepts-criteria/)). A need for treatment does not by itself mean a need for transport.
- Health boards list examples such as regular dialysis or cancer treatment, needing a stretcher or oxygen, travelling in one's own wheelchair, or being unable to walk without continual support ([Betsi Cadwaladr UHB](https://bcuhb.nhs.wales/services/hospital-services/non-emergency-patient-transport-service-nepts/)).
- The Welsh Ambulance Service reported **tightening how eligibility is assessed** ([Llais](https://www.llaiswales.org/news-and-reports/news/public-concerns-over-changes-non-emergency-transport-service)); reports about the date conflict. Patients found ineligible are pointed to an alternative transport team and the NHS travel costs scheme.

## What this means for the PAS

1. **Eligibility is the service's decision, not ours.** The PAS must not tell a patient they qualify. A "cannot travel" barrier on a reply (CAP-14) leads to a *request* a person reviews, never to a booked vehicle.
2. **Verify with the Welsh Ambulance Service** the current criteria, the booking route and who may request on a patient's behalf, and whether a clinic can pre-book. Record the answer here with the date and the person who confirmed it.
3. **Do not hard-code criteria.** If a pre-screen is added, make its questions configuration, owned by the clinical lead, and keep them as a prompt to staff, not a decision.
4. **Alternatives must be offered** where transport is refused (a different time, a video or phone visit, travel-cost help), which is what the reply options and CAP-14 already do.

Open until a provider interface and a named contact exist: NOT-3.
