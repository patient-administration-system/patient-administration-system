-- SeaORM is gone (SQLX-1, then SEAORM-1). A database migrated before sqlx kept SeaORM's record of its migrations
-- in seaql_migrations, which pas-migrate carried into _sqlx_migrations once and never read again. That carrying
-- over is removed with this migration, so the old record goes too. A fresh database never had it.
DROP TABLE IF EXISTS seaql_migrations;
