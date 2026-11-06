DROP TABLE IF EXISTS tolerance_alerts;

DELETE FROM tolerances WHERE scope_kind <> 'all';

DROP INDEX IF EXISTS ux_tolerances_scope;

ALTER TABLE tolerances DROP CONSTRAINT IF EXISTS tolerances_scope_matches_kind, DROP COLUMN IF EXISTS scope_kind, DROP COLUMN IF EXISTS scope_key;

ALTER TABLE tolerances ADD PRIMARY KEY (metric);
