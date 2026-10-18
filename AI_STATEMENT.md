# AI statement

Much of this repository was written with an AI assistant (Anthropic's Claude), working at the maintainer's direction in an interactive coding session, including code, tests, specifications, governance drafts and translations.

## What that means for you

- **Human review is not complete.** The maintainer directs and can read everything, but nobody has independently audited the code or documents. Treat them as you would an unreviewed contribution.
- **Translations** into every language other than English were drafted by AI and have not been reviewed by first-language speakers ([`governance/translation-review.md`](governance/translation-review.md)).
- **Governance documents** (DPIA, hazard log, accessibility statement) are drafts from the code. They are not clinical, legal or privacy advice.
- **Self-assessments**, including the DTAC document, are written to be critical, and say where the product falls short.
- **Tests are the evidence.** Claims are intended to be backed by a test or a command in the repository; where they are not, the text says "not yet tested".

## Where AI is not used

The software contains no machine-learning model and makes no automated decision about a patient. Any future risk score must follow NFR-11 (it may only add support) and needs a fairness audit and clinical safety review first.
