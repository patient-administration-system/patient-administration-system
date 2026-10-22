//! Part of [`super`]: stub.

use super::*;

/// In-memory [`PersonClient`] for tests. Zero HTTP. Stores everything
/// in a `Mutex<HashMap<Uuid, Person>>` so each test starts from a known
/// blank slate.
///
/// Not suitable for production. Production runs use [`HttpPersonClient`].
pub struct StubPersonClient {
    rows: Mutex<HashMap<Uuid, Person>>,
}

impl Default for StubPersonClient {
    fn default() -> Self {
        Self::new()
    }
}

impl StubPersonClient {
    pub fn new() -> Self {
        Self {
            rows: Mutex::new(HashMap::new()),
        }
    }

    /// Pre-seed a person row. Useful for tests that want a known id to
    /// reference before exercising the PAS flow.
    pub fn insert(&self, person: Person) {
        self.rows
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(person.id, person);
    }

    /// Number of person rows currently held.
    pub fn len(&self) -> usize {
        self.rows
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_empty()
    }
}

#[async_trait]
impl PersonClient for StubPersonClient {
    async fn create(&self, person: PersonCreate, _ctx: &UserContext) -> Result<Person> {
        let id = Uuid::new_v4();
        let new = Person {
            id,
            data: person.fields,
        };
        self.rows
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(id, new.clone());
        Ok(new)
    }

    async fn get(&self, id: Uuid, _ctx: &UserContext) -> Result<Person> {
        self.rows
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&id)
            .cloned()
            .ok_or_else(|| Error::not_found(format!("person {id}")))
    }

    async fn update(&self, id: Uuid, person: PersonCreate, _ctx: &UserContext) -> Result<Person> {
        let mut rows = self
            .rows
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !rows.contains_key(&id) {
            return Err(Error::not_found(format!("person {id}")));
        }
        let updated = Person {
            id,
            data: person.fields,
        };
        rows.insert(id, updated.clone());
        Ok(updated)
    }

    async fn delete(&self, id: Uuid, _ctx: &UserContext) -> Result<()> {
        let removed = self
            .rows
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&id);
        match removed {
            Some(_) => Ok(()),
            None => Err(Error::not_found(format!("person {id}"))),
        }
    }

    async fn search(&self, _query: &str, limit: u32, _ctx: &UserContext) -> Result<Vec<Person>> {
        // The stub does not implement full-text matching. It returns up to
        // `limit` arbitrary rows. Tests that care about query semantics
        // assert against the real `person-service`.
        let rows = self
            .rows
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Ok(rows.values().take(limit as usize).cloned().collect())
    }

    async fn match_persons(
        &self,
        _candidate: PersonCreate,
        _ctx: &UserContext,
    ) -> Result<Vec<PersonMatch>> {
        Ok(Vec::new())
    }

    async fn check_duplicates(
        &self,
        _candidate: PersonCreate,
        _ctx: &UserContext,
    ) -> Result<Vec<PersonMatch>> {
        Ok(Vec::new())
    }

    async fn merge(&self, source: Uuid, target: Uuid, _ctx: &UserContext) -> Result<MergeOutcome> {
        let mut rows = self
            .rows
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !rows.contains_key(&source) {
            return Err(Error::not_found(format!("person {source}")));
        }
        if !rows.contains_key(&target) {
            return Err(Error::not_found(format!("person {target}")));
        }
        if source == target {
            return Err(Error::validation("self-merge"));
        }
        rows.remove(&source);
        Ok(MergeOutcome {
            source_id: source,
            target_id: target,
        })
    }

    async fn masked(&self, id: Uuid, ctx: &UserContext) -> Result<Person> {
        self.get(id, ctx).await
    }

    async fn export(&self, id: Uuid, ctx: &UserContext) -> Result<serde_json::Value> {
        let p = self.get(id, ctx).await?;
        serde_json::to_value(p)
            .map_err(|e| Error::internal(format!("export could not be serialized: {e}")))
    }

    async fn audit(
        &self,
        _id: Uuid,
        _limit: u32,
        _ctx: &UserContext,
    ) -> Result<Vec<UpstreamAuditEntry>> {
        Ok(Vec::new())
    }

    async fn bulk_export(&self, format: &str, _ctx: &UserContext) -> Result<(String, Vec<u8>)> {
        if !matches!(format, "json" | "xml" | "tsv" | "csv") {
            return Err(Error::validation(format!(
                "bulk_export: unsupported format {format:?}"
            )));
        }
        let rows: Vec<Person> = self
            .rows
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .values()
            .cloned()
            .collect();
        // The stub returns minimal payloads good enough for tests; the
        // production upstream owns the real formatter logic. JSON is the
        // only format with a structured stub body — the others return
        // an empty payload with the right content-type so handlers can
        // assert the round-trip without depending on upstream format.
        match format {
            "json" => {
                let items = rows
                    .iter()
                    .map(serde_json::to_value)
                    .collect::<std::result::Result<Vec<_>, _>>()
                    .map_err(|e| Error::internal(format!("export could not be serialized: {e}")))?;
                let arr = serde_json::Value::Array(items);
                let body = serde_json::to_vec(&arr).unwrap_or_default();
                Ok(("application/json".into(), body))
            }
            "xml" => Ok(("application/xml".into(), b"<persons/>".to_vec())),
            "tsv" => Ok(("text/tab-separated-values".into(), Vec::new())),
            "csv" => Ok(("text/csv".into(), Vec::new())),
            _ => unreachable!(),
        }
    }

    async fn bulk_import(
        &self,
        _content_type: &str,
        _body: Vec<u8>,
        _ctx: &UserContext,
    ) -> Result<serde_json::Value> {
        Ok(serde_json::json!({ "imported": 0, "errors": [] }))
    }
}
