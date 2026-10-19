//! CAP-11: coarse demographic bands for the privacy-safe analytics feed.
//!
//! Age and deprivation are held by person-service (and an index of deprivation needs a postcode lookup that
//! is not the PAS's job). An upstream job pushes only the coarse bands the analytics need: an age band and a
//! deprivation quintile. The PAS never stores a birth date or postcode for this, and the analytics feed
//! never outputs a person or appointment identifier.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE TABLE IF NOT EXISTS person_demographics ( \
                   person_id uuid PRIMARY KEY, \
                   age_band text NULL CHECK (age_band IS NULL OR age_band IN ('0-15','16-24','25-34','35-44','45-54','55-64','65-74','75+')), \
                   deprivation_quintile smallint NULL CHECK (deprivation_quintile IS NULL OR deprivation_quintile BETWEEN 1 AND 5), \
                   source text NOT NULL CHECK (length(btrim(source)) > 0), \
                   updated_at timestamptz NOT NULL DEFAULT now())",
            )
            .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared("DROP TABLE IF EXISTS person_demographics")
            .await?;
        Ok(())
    }
}
