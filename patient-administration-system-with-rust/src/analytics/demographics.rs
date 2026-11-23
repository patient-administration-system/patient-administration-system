//! Part of [`super`]: demographics.

use super::*;
use crate::db::row::RowExt;
use crate::db::sql::{Conn, Param, Stmt};
use patient_administration_system_domain::arith::in_range;

impl AnalyticsService {
    /// Record an upstream person's coarse bands. Nothing else about them is taken.
    pub async fn set_demographics(
        &self,
        person_id: uuid::Uuid,
        age_band: Option<&str>,
        quintile: Option<i16>,
        source: &str,
        ctx: &UserContext,
    ) -> Result<()> {
        if source.trim().is_empty() {
            return Err(Error::validation(
                "source must say where the bands come from",
            ));
        }
        if let Some(q) = quintile
            && !(1..=5).contains(&q)
        {
            return Err(Error::validation("deprivation_quintile is 1 to 5"));
        }
        self.db
            .run(Stmt::new("INSERT INTO person_demographics (person_id, age_band, deprivation_quintile, source) VALUES ($1,$2,$3,$4) \
                 ON CONFLICT (person_id) DO UPDATE SET age_band = $2, deprivation_quintile = $3, source = $4, updated_at = now()",
                [
                    person_id.into(),
                    age_band.map(String::from).into(),
                    quintile.into(),
                    source.trim().into(),
                ],
            ))
            .await
            .map_err(|e| {
                if crate::db::sql::is_rejected_statement(&e) {
                    Error::validation("age_band must be one of 0-15, 16-24, 25-34, 35-44, 45-54, 55-64, 65-74, 75+")
                } else {
                    e
                }
            })?;
        AuditLogRepository::log(
            &self.db,
            "patient",
            person_id,
            "demographic_bands_set",
            None,
            Some(serde_json::json!({ "source": source.trim() })),
            ctx,
        )
        .await
    }

    /// Load a postcode to quintile lookup the health board supplies, as CSV lines `postcode,quintile` (a header
    /// line is allowed). Replaces any earlier row for the same postcode. Bad lines are counted and skipped,
    /// never guessed at. The PAS ships no data of its own.
    pub async fn load_deprivation_lookup(
        &self,
        csv: &str,
        dataset: &str,
        loaded_by: &str,
        ctx: &UserContext,
    ) -> Result<(u64, u64)> {
        let (dataset, by) = (dataset.trim(), loaded_by.trim());
        if dataset.is_empty() || by.is_empty() {
            return Err(Error::validation(
                "name the dataset (for example WIMD 2019 with the postcode file used) and who loaded it",
            ));
        }
        let (mut loaded, mut rejected) = (0u64, 0u64);
        let rows: Vec<(String, i16)> = csv
            .lines()
            .filter_map(|l| {
                let mut it = l.split(',');
                let (pc, q) = (it.next()?, it.next()?);
                if it.next().is_some() {
                    return None;
                }
                let pc = normalise_postcode(pc);
                let q: i16 = q.trim().parse().ok()?;
                ((1..=5).contains(&q) && is_postcode(&pc)).then_some((pc, q))
            })
            .collect();
        let total = crate::arith::narrow::<u64, _>(
            "rows in the file",
            csv.lines().filter(|l| !l.trim().is_empty()).count(),
        )?;
        for chunk in rows.chunks(1000) {
            let mut sql = String::from(
                "INSERT INTO deprivation_lookup (postcode, quintile, dataset, loaded_by) VALUES ",
            );
            let mut values: Vec<Param> = Vec::new();
            for (i, (pc, q)) in chunk.iter().enumerate() {
                if i > 0 {
                    sql.push(',');
                }
                let b = values.len();
                #[expect(
                    clippy::arithmetic_side_effects,
                    reason = "RCA-EXC-2: `b` is the length of a parameter list built from one chunk of rows, four per row, far below usize::MAX"
                )]
                sql.push_str(&format!("(${},${},${},${})", b + 1, b + 2, b + 3, b + 4));
                values.extend([pc.clone().into(), (*q).into(), dataset.into(), by.into()]);
            }
            sql.push_str(" ON CONFLICT (postcode) DO UPDATE SET quintile = EXCLUDED.quintile, dataset = EXCLUDED.dataset, loaded_by = EXCLUDED.loaded_by, loaded_at = now()");
            self.db.run(Stmt::new(sql, values)).await?;
            loaded = crate::arith::add(
                "rows loaded",
                loaded,
                crate::arith::narrow::<u64, _>("rows loaded", chunk.len())?,
            )?;
        }
        // A header line is not a rejection; anything else that did not parse is.
        let header = csv
            .lines()
            .next()
            .is_some_and(|l| l.to_lowercase().contains("postcode"));
        // More lines parsed than were read leaves none rejected.
        rejected = crate::arith::add(
            "rows rejected",
            rejected,
            total
                .checked_sub(loaded.checked_add(u64::from(header)).unwrap_or(u64::MAX))
                .unwrap_or(0),
        )?;
        AuditLogRepository::log(
            &self.db,
            "deprivation_lookup",
            uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, dataset.as_bytes()),
            "deprivation_lookup_loaded",
            None,
            Some(serde_json::json!({ "dataset": dataset, "loaded_by": by, "loaded": loaded, "rejected": rejected })),
            ctx,
        )
        .await?;
        Ok((loaded, rejected))
    }

    /// Fill `person_demographics` for people who have appointments and no bands yet (or stale ones): the age
    /// band from the date of birth and the quintile from the postcode, looked up in the board's own table.
    /// Only the two coarse bands are kept; the date of birth and postcode are read, used and not stored.
    /// A person with no usable date or no matching postcode gets the part that is known. Returns how many
    /// people were updated.
    #[expect(
        clippy::arithmetic_side_effects,
        reason = "RCA-EXC-2: each counter here rises by one per item of a collection already in memory, or per event handled, so it cannot reach the maximum of its type"
    )]
    pub async fn refresh_demographics(
        &self,
        people: &dyn crate::upstream::PersonClient,
        limit: i64,
        upstream_ctx: &crate::upstream::UserContext,
        ctx: &UserContext,
    ) -> Result<u64> {
        let limit = in_range("limit", limit, 1, 5000)?;
        let ids = self
            .db
            .all(Stmt::new("SELECT a.person_id FROM appointments a \
                 LEFT JOIN person_demographics d ON d.person_id = a.person_id \
                 WHERE a.deleted_at IS NULL AND (d.person_id IS NULL OR d.updated_at < now() - interval '90 days') \
                 GROUP BY a.person_id ORDER BY max(a.updated_at) DESC LIMIT $1",
                [limit.into()],
            ))
.await?;
        let mut n: u64 = 0;
        for r in ids {
            let id: uuid::Uuid = r.col("person_id")?;
            // A person the upstream cannot return is skipped, not an error for the whole run.
            let Ok(person) = people.get(id, upstream_ctx).await else {
                continue;
            };
            let band = person
                .data
                .get("birth_date")
                .or_else(|| person.data.get("birthDate"))
                .and_then(|v| v.as_str())
                .and_then(|d| d.parse::<jiff::civil::Date>().ok())
                .map(|d| {
                    age_band(
                        d,
                        Timestamp::now()
                            .to_zoned_datetime(jiff::tz::TimeZone::UTC)
                            .date(),
                    )
                });
            let postcode = person
                .data
                .get("addresses")
                .and_then(|a| a.as_array())
                .and_then(|a| a.first())
                .and_then(|a| a.get("postal_code").or_else(|| a.get("postalCode")))
                .and_then(|v| v.as_str())
                .map(normalise_postcode);
            let (quintile, dataset): (Option<i16>, Option<String>) = match postcode {
                Some(pc) => self
                    .db
                    .one(Stmt::new(
                        "SELECT quintile, dataset FROM deprivation_lookup WHERE postcode = $1",
                        [pc.into()],
                    ))
                    .await?
                    // Both columns are nullable: NULL is `None`; an unreadable column is an error.
                    .map(|r| Ok::<_, Error>((r.col("quintile")?, r.col("dataset")?)))
                    .transpose()?
                    .unwrap_or((None, None)),
                None => (None, None),
            };
            if band.is_none() && quintile.is_none() {
                continue;
            }
            let source = format!("refresh:{}", dataset.as_deref().unwrap_or("age-only"));
            self.set_demographics(id, band, quintile, &source, ctx)
                .await?;
            n += 1;
        }
        Ok(n)
    }
}
