# 1. Purpose and vision

The Patient Administration System (PAS) is the administrative core of a hospital: who is admitted where, who is booked when, who is waiting for what, and what must be sent to whom. It records those facts once, audits every change, and publishes each change as an event so other systems stay in step.

Three principles shape every decision:

1. **Administrative, not clinical.** The PAS holds no diagnosis, dose or care plan and decides no treatment. Where an administrative fact can affect safety (a wrong bed, a lost appointment), the system makes the wrong state hard to reach and the history impossible to erase.
2. **Identity is delegated.** Who a patient or worker *is* belongs to `person-service` and `worker-service`. The PAS stores their ids and the facts it owns.
3. **Patients are users too.** A patient can sign in and see their own appointments and messages, and nothing else.

The vision for appointments is that fewer people miss care they need, and that no group is made worse off by the effort. See [`../plan.md`](../plan.md) §3 and requirement NFR-11.
