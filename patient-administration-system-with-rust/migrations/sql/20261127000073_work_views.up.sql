-- TSC-7: a named filter of the work queue, saved by a person and shared with a role.

CREATE TABLE IF NOT EXISTS work_views (
    id uuid PRIMARY KEY,
    name text NOT NULL CHECK (char_length(btrim(name)) BETWEEN 1 AND 100),
    created_by text NOT NULL CHECK (char_length(btrim(created_by)) BETWEEN 1 AND 200),
    queue text NULL CHECK (queue IS NULL OR queue IN ('booking','communication','operations','validation')),
    owner_filter text NULL CHECK (owner_filter IS NULL OR char_length(btrim(owner_filter)) BETWEEN 1 AND 200),
    priority smallint NULL CHECK (priority IS NULL OR priority BETWEEN 1 AND 3),
    overdue boolean NOT NULL DEFAULT false,
    kind text NULL CHECK (kind IS NULL OR kind IN ('call','follow_up','validation','needs_review','tolerance_alert','unreached')),
    shared_with_role text NULL CHECK (shared_with_role IS NULL OR char_length(btrim(shared_with_role)) BETWEEN 1 AND 100),
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (created_by, name)
);

CREATE INDEX IF NOT EXISTS work_views_shared_idx ON work_views (shared_with_role) WHERE shared_with_role IS NOT NULL;
