//! Carer and proxy authority (NEW-7). A proxy is another person (held by person-service) who may act for a
//! patient within scopes, on a recorded basis, until an expiry, unless revoked.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        for sql in [
            "CREATE TABLE IF NOT EXISTS proxy_authorities ( \
               id uuid PRIMARY KEY, \
               person_id uuid NOT NULL, \
               proxy_person_id uuid NOT NULL CHECK (proxy_person_id <> person_id), \
               relationship text NOT NULL CHECK (length(btrim(relationship)) BETWEEN 1 AND 60), \
               basis text NOT NULL CHECK (basis IN \
                 ('patient_consent','parental_responsibility','lasting_power_of_attorney','court_appointed','other')), \
               scopes text[] NOT NULL CHECK (cardinality(scopes) > 0 AND scopes <@ ARRAY['receive','confirm','cancel','rebook']), \
               recorded_by text NOT NULL CHECK (length(btrim(recorded_by)) > 0), \
               recorded_at timestamptz NOT NULL, \
               expires_at timestamptz NOT NULL CHECK (expires_at > recorded_at), \
               revoked_by text, \
               revoked_at timestamptz, \
               CHECK ((revoked_at IS NULL) = (revoked_by IS NULL)))",
            "CREATE INDEX IF NOT EXISTS idx_proxy_authorities_pair ON proxy_authorities (person_id, proxy_person_id)",
        ] {
            db.execute_unprepared(sql).await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared("DROP TABLE IF EXISTS proxy_authorities")
            .await?;
        Ok(())
    }
}
