-- Rows for the restore drill (QUAL-23), so it compares data and not an empty schema ("0 = 0").
-- Synthetic and free of personal data; for a throwaway CI database only. Idempotent.
INSERT INTO schedules (id, owner_kind, owner_id, service_type, active, created_at, updated_at)
SELECT ('00000000-0000-4000-8000-' || lpad(i::text, 12, '0'))::uuid, 'Practitioner', gen_random_uuid(),
       'drill-' || i, i % 2 = 0, now(), now()
  FROM generate_series(1, 5) i
ON CONFLICT (id) DO NOTHING;

INSERT INTO slots (id, schedule_id, start_datetime, end_datetime, status, created_at, updated_at)
SELECT gen_random_uuid(), ('00000000-0000-4000-8000-' || lpad((1 + i % 5)::text, 12, '0'))::uuid,
       timestamptz '2030-01-01 09:00Z' + i * interval '30 minutes',
       timestamptz '2030-01-01 09:30Z' + i * interval '30 minutes', 'free', now(), now()
  FROM generate_series(1, 50) i
 WHERE NOT EXISTS (SELECT 1 FROM slots s WHERE s.schedule_id = '00000000-0000-4000-8000-000000000001');

INSERT INTO audit_log (id, entity_type, entity_id, action, old_value, new_value, user_id, at, reason_code, context)
SELECT gen_random_uuid(), 'drill', gen_random_uuid(), 'create', NULL,
       jsonb_build_object('n', i, 'text', 'café ' || i), 'restore-drill', now(), 'drill', '{"seed": true}'
  FROM generate_series(1, 20) i
 WHERE NOT EXISTS (SELECT 1 FROM audit_log WHERE user_id = 'restore-drill');

INSERT INTO tolerances (metric, target, tolerance, escalate_beyond, set_by, note, scope_kind, scope_key)
VALUES ('drill_metric', 0.85, 0.05, 0.15, 'restore-drill', 'seeded for the drill', 'all', '')
ON CONFLICT (metric, scope_kind, scope_key) DO NOTHING;
