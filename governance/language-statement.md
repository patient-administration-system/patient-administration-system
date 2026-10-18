# Language statement (DRAFT)

## Languages

Arabic (`ar-001`), Welsh (`cy-001`), German (`de-001`, `de-de`), English (`en-001`, the source), Spanish (`es-001`), French (`fr-001`), Hindi (`hi-001`) and Simplified Chinese (`zh-001`, `zh-cn`).

## Honest status

**All translations other than English are machine-drafted and have not been reviewed by a first-language speaker.** See [`translation-review.md`](translation-review.md). Welsh in particular must be reviewed by a first-language Welsh speaker before it is shown to patients. A health board with Welsh-language duties should treat the Welsh catalogue as a draft until that review is recorded.

## What is translated

The UI's own text: navigation, headings, table headers, statuses, priorities, messages, and the calendar's labels. Not translated: text that comes from other systems (service names, reasons typed by staff, message templates written by administrators, the API's own error text).

## Message templates

Reminder and letter templates are content, edited by administrators, and today exist in one language each. A Welsh/English pair per template, with a check that blocks a template that exists in one language only, is tracked as CAP-9 in `tasks.md`.
