# Differencing across analytics queries (CAP-11): what is and is not prevented

Each answer from the analytics feed hides cells under k (10, never below 5) and, if exactly one cell is hidden, hides the smallest other cell too, so a single answer cannot be subtracted from its own total.

**Not prevented:** someone who may ask many differently shaped questions can subtract one answer from another. A window that differs by a week, or a split that adds a dimension, can recover a cell that was hidden. No rule on one answer fixes that for a free-form feed.

## What the PAS does about it

- **Bounded:** each caller has a daily query budget (`PAS_ANALYTICS_QUERY_BUDGET`, default 100). After that the feed refuses until their oldest query is 24 hours old.
- **Visible:** every query is recorded in the audit log (`analytics_query`) with its shape (dimensions, window, k), so unusual probing, such as the same report with the window shifted day by day, can be seen and asked about.

## What only people can do

- Give the feed only to those who need it, and say in the data-sharing terms that **re-identifying or reconstructing small cells is prohibited** and will be investigated.
- Prefer a **fixed publication** (a monthly report with agreed splits, produced once and shared) to open access, where it meets the need. Fixed outputs can be checked as a set.
- Review the audit-log query shapes now and then, and lower the budget for those who do not need much.

If these terms cannot be agreed, treat the feed as internal only.
