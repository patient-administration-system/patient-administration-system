-- One staff work queue (TSC-1). Each open item of an existing list (a call to make, a follow-up to chase, a
-- waiting-list entry to validate, communication needs to review, a tolerance alert) has one row here, with an
-- owner, a priority and a due date; the row closes when its source closes. Support only: a work item never
-- changes an appointment, a referral or a pathway.
CREATE TABLE IF NOT EXISTS work_items (
    id uuid PRIMARY KEY,
    kind text NOT NULL CHECK (kind IN ('call','follow_up','validation','needs_review','tolerance_alert')),
    source_id uuid NOT NULL,
    person_id uuid NULL,
    subject_kind text NULL CHECK (subject_kind IS NULL OR subject_kind IN ('appointment','pathway','profile','alert')),
    subject_id uuid NULL,
    queue text NOT NULL CHECK (length(btrim(queue)) > 0),
    priority smallint NOT NULL CHECK (priority BETWEEN 1 AND 3),
    due_at timestamptz NOT NULL,
    status text NOT NULL DEFAULT 'open' CHECK (status IN ('open','claimed','closed')),
    owner text NULL CHECK (owner IS NULL OR length(btrim(owner)) > 0),
    opened_at timestamptz NOT NULL DEFAULT now(),
    claimed_at timestamptz NULL,
    closed_at timestamptz NULL,
    UNIQUE (kind, source_id),
    -- A claimed item has an owner; an open one has none; a closed one keeps whoever had it.
    CHECK (status <> 'claimed' OR (owner IS NOT NULL AND claimed_at IS NOT NULL)),
    CHECK (status <> 'open' OR owner IS NULL),
    CHECK ((status = 'closed') = (closed_at IS NOT NULL))
);

CREATE INDEX IF NOT EXISTS idx_work_items_queue ON work_items (queue, status, priority, due_at) WHERE status <> 'closed';
CREATE INDEX IF NOT EXISTS idx_work_items_owner ON work_items (owner) WHERE status = 'claimed';
