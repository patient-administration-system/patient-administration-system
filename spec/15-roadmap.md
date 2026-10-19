# 15. Roadmap

The order and reasoning are in [`../plan.md`](../plan.md) §6. In short:

1. Make failures visible: CI with one gate, database tests that cannot skip, supply-chain policy.
2. Make it honest to run: licence file, security policy, real API authentication, MFA.
3. Make notifications real and safe: consent and opt-out before carriers; then receipts, cadence, two-way replies.
4. Make it accessible and multilingual: WCAG 2.2 AA, translations (Welsh first).
5. Documents that need people: clinical safety, DPIA, penetration test, usability testing.
6. Improvements that need data: coded outcomes, KPIs, backfill, and only after a fairness audit, a support-only risk score.

**Position (2026-10-20).** Steps 1 to 4 and the code for step 6 are built. What is left of steps 2, 3 and 5 needs people or contracts: a Clinical Safety Officer, DPIA sign-off, carrier contracts, a penetration test, usability testing, and fluent translation review. The no-show score is built but off; its rollout design is `governance/noshow-evaluation.md`.

