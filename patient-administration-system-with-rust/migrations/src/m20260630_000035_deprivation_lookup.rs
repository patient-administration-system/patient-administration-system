//! The measurement feed (CAP-11, CAS-7, CAP-12 audit): a postcode to deprivation-quintile lookup.
//!
//! `person_demographics` (age band, deprivation quintile) had nothing to fill it. This table holds the
//! lookup a health board supplies, derived from a postcode-to-area file and the area-level deprivation index
//! it is licensed to use (in Wales, WIMD). The PAS ships **no data**: the board loads its own, naming the
//! dataset and who loaded it. See `governance/deprivation-data-route.md`.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE TABLE IF NOT EXISTS deprivation_lookup ( \
                   postcode text PRIMARY KEY CHECK (postcode = upper(postcode) AND postcode !~ '\\s'), \
                   quintile smallint NOT NULL CHECK (quintile BETWEEN 1 AND 5), \
                   dataset text NOT NULL CHECK (length(btrim(dataset)) > 0), \
                   loaded_by text NOT NULL CHECK (length(btrim(loaded_by)) > 0), \
                   loaded_at timestamptz NOT NULL DEFAULT now())",
            )
            .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared("DROP TABLE IF EXISTS deprivation_lookup")
            .await?;
        Ok(())
    }
}
