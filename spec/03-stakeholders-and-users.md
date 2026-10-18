# 3. Stakeholders and users

| Role | Keycloak role | What they do |
|---|---|---|
| Patient | `patient` | Signs in; reads their own appointments and portal messages |
| Staff | `staff` | Calendar, waitlist, schedule, patient search |
| Administrator | `admin` (also has staff access) | As staff, plus editing message templates |
| Integrating system | API bearer token | Calls the REST API; receives events |
| Clinical Safety Officer | n/a (not yet named) | Owns the hazard log (see DTAC-1) |
| Deploying organisation | n/a | Hosts, holds DSPT/ICO/Cyber Essentials, owns its DPIA |

A patient is linked to a record by the `person_id` attribute on their identity-provider account. That link is set by an administrator after identity checks made outside this system (see §16, question 2).
