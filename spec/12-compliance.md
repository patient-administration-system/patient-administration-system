# 12. Compliance

Honest status. The full assessment against the NHS Digital Technology Assessment Criteria v2.0 is [`docs/united-kingdom/national-health-service/digital-technology-assessment-criteria/2.0/index.md`](../docs/united-kingdom/national-health-service/digital-technology-assessment-criteria/2.0/index.md); it concludes the product would not pass today.

| Regime | Position |
|---|---|
| UK GDPR / Data Protection Act 2018 | Technical controls exist (audit, consent records, subject-access export, masking). DPIA and privacy notice: not written. See [`governance/`](../governance/dpia.md) |
| HL7 FHIR R5 (`/fhir`) | Partly. The CapabilityStatement validates against base 5.0.0; the Appointment, Encounter, Slot and Schedule reads do not (they return internal field names). See QUAL-39 and the R5 note in [`09-api-surface.md`](09-api-surface.md) |
| ISO 14971 risk management | Not done, and may not apply (not judged a medical device). Mapping and gaps: [`governance/risk-management-iso-14971.md`](../governance/risk-management-iso-14971.md) |
| DCB0129 / DCB0160 clinical safety | Not done. No Clinical Safety Officer or hazard log. See [`governance/clinical-safety.md`](../governance/clinical-safety.md) |
| DTAC | Self-assessed; fails C1, C2, C3 on missing artefacts |
| WCAG 2.2 AA | Automated checks only (NFR-6); no manual audit or published statement |
| PECR / consent to message | Not built (NFR-15, tasks.md NOT-5) |
| Data location | Decided by the deployer; the software calls no non-UK service at run time unless configured |

## Personal data

Every column holding personal data MUST appear in [`governance/personal-data-inventory.csv`](../governance/personal-data-inventory.csv). A test fails when one does not (NFR-12).

## Retention

See [`governance/retention-schedule.md`](../governance/retention-schedule.md). Audit rows are append-only and never deleted by the application.
