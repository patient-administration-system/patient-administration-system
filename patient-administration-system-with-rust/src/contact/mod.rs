//! Reliable contact details and communication needs (CAP-4).
//!
//! Demographics live in `person-service`. What the PAS owns, keyed by `person_id`, is how to reach
//! a patient *well*: a validated mobile number and when it was last verified, an alternate or carer
//! contact, a preferred channel and language (the "active offer" of care in the patient's own
//! language), and accessibility needs. Other systems read it as JSON or as a FHIR `Patient`
//! fragment.
//!
//! Two rules matter more than the fields:
//!
//! - **A number is trusted only if it is verified and recent.** Changing the number clears the
//!   verification, so a new, unproven number is never mistaken for a checked one. Reminders use a
//!   profile number only when it passes this test, otherwise they fall back to what the upstream
//!   record holds.
//! - **"Validated" means well-formed, not owned.** Validation checks the international (E.164)
//!   shape and length; it cannot know the number exists, is a mobile, or belongs to the patient.
//!   Ownership is what `verify_mobile` records, and the PAS does not itself send the verification
//!   code (that needs a carrier, NOT-3).

use chrono::{DateTime, Duration, Utc};
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, FromQueryResult, Statement};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::db::repositories::audit::{AuditLogRepository, UserContext};
use crate::{Error, Result};

/// A mobile number older than this without verification is stale. A year is a starting point; the
/// deploying organisation sets its own with `?older_than_days=`.
pub const DEFAULT_STALE_DAYS: i64 = 365;

pub const ACCESSIBILITY_NEEDS: [&str; 5] = [
    "easy_read",
    "large_print",
    "audio",
    "british_sign_language",
    // No digital message of any kind: letters only (CAS-10). Enforced where every message is generated.
    "letter_only",
];
pub const CHANNELS: [&str; 6] = [
    "print",
    "email",
    "sms",
    "whatsapp",
    "apple_messages",
    "portal",
];

fn db_err(e: sea_orm::DbErr) -> Error {
    Error::Database(e)
}

// --- pure rules (unit tested) -----------------------------------------------------------------

/// Normalise a phone number to E.164, or explain why it is not one.
///
/// Spaces, dashes, dots and brackets are removed; a leading `00` means `+`. A national number (a
/// leading `0`) is accepted only when `default_country_code` (digits, no `+`) is supplied, in which
/// case the `0` is replaced by it. The result is `+` followed by 8 to 15 digits, the first not 0.
pub fn normalise_e164(raw: &str, default_country_code: Option<&str>) -> Result<String> {
    let mut s: String = raw
        .chars()
        .filter(|c| !matches!(c, ' ' | '-' | '.' | '(' | ')'))
        .collect();
    if let Some(rest) = s.strip_prefix("00") {
        s = format!("+{rest}");
    } else if let Some(rest) = s.strip_prefix('0') {
        let cc = default_country_code
            .filter(|c| !c.is_empty() && c.chars().all(|d| d.is_ascii_digit()))
            .ok_or_else(|| {
                Error::validation(
                    "a national number needs a country code: use +<country code><number>",
                )
            })?;
        s = format!("+{cc}{rest}");
    }
    let digits = s
        .strip_prefix('+')
        .ok_or_else(|| Error::validation("a phone number must start with + and a country code"))?;
    if !digits.chars().all(|c| c.is_ascii_digit()) {
        return Err(Error::validation(
            "a phone number may contain only digits after the +",
        ));
    }
    if !(8..=15).contains(&digits.len()) || digits.starts_with('0') {
        return Err(Error::validation(
            "a phone number must have 8 to 15 digits and a country code that does not start with 0",
        ));
    }
    Ok(s)
}

/// The BCP 47 form of a catalogue locale: `cy-001` is `cy` (the 001 "world" region carries no
/// information), `de-de` is `de-DE`.
pub fn bcp47(locale: &str) -> String {
    match locale.split_once('-') {
        Some((lang, "001")) => lang.to_string(),
        Some((lang, region)) => format!("{lang}-{}", region.to_uppercase()),
        None => locale.to_string(),
    }
}

/// True when the number is absent, never verified, or last verified more than `max_age_days` ago.
pub fn is_stale(
    mobile: Option<&str>,
    verified_at: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
    max_age_days: i64,
) -> bool {
    match (mobile, verified_at) {
        (None, _) => true,
        (Some(_), None) => true,
        (Some(_), Some(at)) => now - at > Duration::days(max_age_days),
    }
}

// --- the profile ------------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, FromQueryResult)]
pub struct ProfileRow {
    pub person_id: Uuid,
    pub mobile_e164: Option<String>,
    pub mobile_verified_at: Option<DateTime<Utc>>,
    pub mobile_verification_method: Option<String>,
    pub alternate_name: Option<String>,
    pub alternate_relationship: Option<String>,
    pub alternate_phone_e164: Option<String>,
    pub alternate_email: Option<String>,
    pub alternate_is_carer: bool,
    pub alternate_is_emergency: bool,
    pub preferred_channel: Option<String>,
    pub preferred_language: Option<String>,
    pub accessibility_needs: Vec<String>,
    pub updated_at: DateTime<Utc>,
    pub updated_by: Option<String>,
}

const COLS: &str = "person_id, mobile_e164, mobile_verified_at, mobile_verification_method, alternate_name, \
    alternate_relationship, alternate_phone_e164, alternate_email, alternate_is_carer, alternate_is_emergency, \
    preferred_channel, preferred_language, accessibility_needs, updated_at, updated_by";

/// What a caller may set. The verification state is deliberately not here.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ProfileInput {
    #[serde(default)]
    pub mobile: Option<String>,
    #[serde(default)]
    pub alternate_name: Option<String>,
    #[serde(default)]
    pub alternate_relationship: Option<String>,
    #[serde(default)]
    pub alternate_phone: Option<String>,
    #[serde(default)]
    pub alternate_email: Option<String>,
    #[serde(default)]
    pub alternate_is_carer: bool,
    #[serde(default)]
    pub alternate_is_emergency: bool,
    #[serde(default)]
    pub preferred_channel: Option<String>,
    #[serde(default)]
    pub preferred_language: Option<String>,
    #[serde(default)]
    pub accessibility_needs: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Profile {
    #[serde(flatten)]
    pub row: ProfileRow,
    /// True when the mobile is missing, unverified, or last verified more than a year ago.
    pub mobile_stale: bool,
    /// The same facts as a FHIR `Patient` fragment, for systems that speak FHIR.
    pub fhir: serde_json::Value,
}

impl Profile {
    fn from_row(row: ProfileRow, now: DateTime<Utc>) -> Self {
        let mobile_stale = is_stale(
            row.mobile_e164.as_deref(),
            row.mobile_verified_at,
            now,
            DEFAULT_STALE_DAYS,
        );
        let fhir = fhir_fragment(&row);
        Profile {
            row,
            mobile_stale,
            fhir,
        }
    }
}

/// A FHIR R5 `Patient` fragment: the mobile as a `telecom` with when it was verified, the preferred
/// language as `communication`, the alternate as `contact`, and accessibility needs as an extension.
pub fn fhir_fragment(p: &ProfileRow) -> serde_json::Value {
    let mut telecom = Vec::new();
    if let Some(m) = &p.mobile_e164 {
        let mut t = serde_json::json!({ "system": "phone", "value": m, "use": "mobile" });
        if let Some(at) = p.mobile_verified_at {
            t["extension"] = serde_json::json!([{
                "url": "https://patient-administration-system.example/fhir/StructureDefinition/telecom-verified-at",
                "valueDateTime": at,
            }]);
        }
        telecom.push(t);
    }
    let mut out =
        serde_json::json!({ "resourceType": "Patient", "id": p.person_id, "telecom": telecom });
    if let Some(lang) = &p.preferred_language {
        out["communication"] = serde_json::json!([{
            "language": { "coding": [{ "system": "urn:ietf:bcp:47", "code": bcp47(lang) }] },
            "preferred": true,
        }]);
    }
    if p.alternate_name.is_some() || p.alternate_phone_e164.is_some() || p.alternate_email.is_some()
    {
        let mut rel = Vec::new();
        if p.alternate_is_emergency {
            rel.push(serde_json::json!({ "coding": [{ "system": "http://terminology.hl7.org/CodeSystem/v2-0131", "code": "C", "display": "Emergency Contact" }] }));
        }
        if p.alternate_is_carer {
            rel.push(serde_json::json!({ "text": "carer" }));
        }
        if let Some(r) = &p.alternate_relationship {
            rel.push(serde_json::json!({ "text": r }));
        }
        let mut tel = Vec::new();
        if let Some(ph) = &p.alternate_phone_e164 {
            tel.push(serde_json::json!({ "system": "phone", "value": ph }));
        }
        if let Some(e) = &p.alternate_email {
            tel.push(serde_json::json!({ "system": "email", "value": e }));
        }
        out["contact"] = serde_json::json!([{ "relationship": rel, "name": { "text": p.alternate_name }, "telecom": tel }]);
    }
    if !p.accessibility_needs.is_empty() {
        out["extension"] = serde_json::json!([{
            "url": "https://patient-administration-system.example/fhir/StructureDefinition/accessibility-needs",
            "valueCodeableConcept": { "coding": p.accessibility_needs.iter().map(|n| serde_json::json!({ "code": n })).collect::<Vec<_>>() },
        }]);
    }
    out
}

fn clean(s: &Option<String>, max: usize, what: &str) -> Result<Option<String>> {
    match s.as_deref().map(str::trim) {
        None | Some("") => Ok(None),
        Some(v) if v.len() > max => Err(Error::validation(format!(
            "{what} is too long (at most {max} characters)"
        ))),
        Some(v) if v.chars().any(char::is_control) => Err(Error::validation(format!(
            "{what} may not contain control characters"
        ))),
        Some(v) => Ok(Some(v.to_string())),
    }
}

/// Validate and normalise an input into the values to store.
pub fn validate(input: &ProfileInput, default_cc: Option<&str>) -> Result<Validated> {
    let mobile = input
        .mobile
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|m| normalise_e164(m, default_cc))
        .transpose()?;
    let alternate_phone = input
        .alternate_phone
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|m| normalise_e164(m, default_cc))
        .transpose()?;
    if let Some(c) = &input.preferred_channel
        && !CHANNELS.contains(&c.as_str())
    {
        return Err(Error::validation(format!(
            "preferred_channel must be one of {}",
            CHANNELS.join(", ")
        )));
    }
    if let Some(l) = &input.preferred_language
        && !(2..=12).contains(&l.len())
    {
        return Err(Error::validation(
            "preferred_language must be a locale code such as cy-001",
        ));
    }
    for n in &input.accessibility_needs {
        if !ACCESSIBILITY_NEEDS.contains(&n.as_str()) {
            return Err(Error::validation(format!(
                "accessibility need {n:?} is not one of {}",
                ACCESSIBILITY_NEEDS.join(", ")
            )));
        }
    }
    let mut needs = input.accessibility_needs.clone();
    needs.sort();
    needs.dedup();
    if let Some(e) = input
        .alternate_email
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        && (!e.contains('@') || e.len() > 254 || e.contains(char::is_whitespace))
    {
        return Err(Error::validation("alternate_email is not an email address"));
    }
    Ok(Validated {
        mobile,
        alternate_name: clean(&input.alternate_name, 120, "alternate_name")?,
        alternate_relationship: clean(&input.alternate_relationship, 60, "alternate_relationship")?,
        alternate_phone,
        alternate_email: clean(&input.alternate_email, 254, "alternate_email")?,
        needs,
    })
}

pub struct Validated {
    mobile: Option<String>,
    alternate_name: Option<String>,
    alternate_relationship: Option<String>,
    alternate_phone: Option<String>,
    alternate_email: Option<String>,
    needs: Vec<String>,
}

// --- the service ------------------------------------------------------------------------------

pub struct ContactService {
    db: DatabaseConnection,
    default_country_code: Option<String>,
}

impl ContactService {
    pub fn new(db: DatabaseConnection) -> Self {
        Self {
            db,
            default_country_code: std::env::var("PAS_DEFAULT_COUNTRY_CODE")
                .ok()
                .filter(|s| !s.is_empty()),
        }
    }

    pub async fn get(&self, person_id: Uuid) -> Result<Option<Profile>> {
        let row = ProfileRow::find_by_statement(Statement::from_sql_and_values(
            DbBackend::Postgres,
            format!("SELECT {COLS} FROM patient_communication_profiles WHERE person_id = $1"),
            [person_id.into()],
        ))
        .one(&self.db)
        .await
        .map_err(db_err)?;
        Ok(row.map(|r| Profile::from_row(r, Utc::now())))
    }

    /// Create or replace a patient's profile. Changing the mobile number clears its verification.
    pub async fn put(
        &self,
        person_id: Uuid,
        input: ProfileInput,
        ctx: &UserContext,
    ) -> Result<Profile> {
        let v = validate(&input, self.default_country_code.as_deref())?;
        let before = self.get(person_id).await?;
        let mobile_changed =
            before.as_ref().map(|p| p.row.mobile_e164.clone()) != Some(v.mobile.clone());
        let keep_verification = before.is_some() && !mobile_changed;
        let row = ProfileRow::find_by_statement(Statement::from_sql_and_values(
            DbBackend::Postgres,
            format!(
                "INSERT INTO patient_communication_profiles (person_id, mobile_e164, alternate_name, alternate_relationship, \
                   alternate_phone_e164, alternate_email, alternate_is_carer, alternate_is_emergency, preferred_channel, \
                   preferred_language, accessibility_needs, updated_by) \
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12) \
                 ON CONFLICT (person_id) DO UPDATE SET \
                   mobile_e164 = EXCLUDED.mobile_e164, \
                   mobile_verified_at = CASE WHEN $13 THEN patient_communication_profiles.mobile_verified_at ELSE NULL END, \
                   mobile_verification_method = CASE WHEN $13 THEN patient_communication_profiles.mobile_verification_method ELSE NULL END, \
                   alternate_name = EXCLUDED.alternate_name, alternate_relationship = EXCLUDED.alternate_relationship, \
                   alternate_phone_e164 = EXCLUDED.alternate_phone_e164, alternate_email = EXCLUDED.alternate_email, \
                   alternate_is_carer = EXCLUDED.alternate_is_carer, alternate_is_emergency = EXCLUDED.alternate_is_emergency, \
                   preferred_channel = EXCLUDED.preferred_channel, preferred_language = EXCLUDED.preferred_language, \
                   accessibility_needs = EXCLUDED.accessibility_needs, updated_at = now(), updated_by = EXCLUDED.updated_by \
                 RETURNING {COLS}"
            ),
            [
                person_id.into(),
                v.mobile.into(),
                v.alternate_name.into(),
                v.alternate_relationship.into(),
                v.alternate_phone.into(),
                v.alternate_email.into(),
                input.alternate_is_carer.into(),
                input.alternate_is_emergency.into(),
                input.preferred_channel.clone().into(),
                input.preferred_language.clone().into(),
                v.needs.into(),
                ctx.user_id.clone().into(),
                keep_verification.into(),
            ],
        ))
        .one(&self.db)
        .await
        .map_err(db_err)?
        .ok_or_else(|| Error::internal("profile upsert returned no row"))?;
        AuditLogRepository::log(
            &self.db,
            "communication_profile",
            person_id,
            if before.is_some() { "update" } else { "create" },
            None,
            Some(serde_json::json!({ "mobile_changed": mobile_changed, "preferred_channel": row.preferred_channel, "preferred_language": row.preferred_language })),
            ctx,
        )
        .await?;
        Ok(Profile::from_row(row, Utc::now()))
    }

    /// Record that the mobile number has been checked (by staff, by the patient in the portal, or by
    /// a code sent to the number). The PAS does not send the code itself.
    pub async fn verify_mobile(
        &self,
        person_id: Uuid,
        method: &str,
        ctx: &UserContext,
    ) -> Result<Profile> {
        if !matches!(method, "staff" | "portal" | "sms_code") {
            return Err(Error::validation(
                "method must be staff, portal or sms_code",
            ));
        }
        let row = ProfileRow::find_by_statement(Statement::from_sql_and_values(
            DbBackend::Postgres,
            format!(
                "UPDATE patient_communication_profiles SET mobile_verified_at = now(), mobile_verification_method = $2, \
                   updated_at = now(), updated_by = $3 WHERE person_id = $1 AND mobile_e164 IS NOT NULL RETURNING {COLS}"
            ),
            [person_id.into(), method.into(), ctx.user_id.clone().into()],
        ))
        .one(&self.db)
        .await
        .map_err(db_err)?
        .ok_or_else(|| Error::not_found("no mobile number on file for that patient"))?;
        AuditLogRepository::log(
            &self.db,
            "communication_profile",
            person_id,
            "verify_mobile",
            None,
            Some(serde_json::json!({ "method": method })),
            ctx,
        )
        .await?;
        Ok(Profile::from_row(row, Utc::now()))
    }

    /// Profiles whose mobile is missing, unverified or last verified more than `older_than_days`
    /// ago: the list a team works through to fix contact details, oldest first.
    pub async fn stale(&self, older_than_days: i64, limit: i64) -> Result<Vec<Profile>> {
        if !(1..=3650).contains(&older_than_days) {
            return Err(Error::validation("older_than_days must be 1 to 3650"));
        }
        let rows = ProfileRow::find_by_statement(Statement::from_sql_and_values(
            DbBackend::Postgres,
            format!(
                "SELECT {COLS} FROM patient_communication_profiles \
                 WHERE mobile_e164 IS NULL OR mobile_verified_at IS NULL OR mobile_verified_at < now() - ($1 || ' days')::interval \
                 ORDER BY mobile_verified_at ASC NULLS FIRST, person_id LIMIT $2"
            ),
            [older_than_days.to_string().into(), limit.clamp(1, 1000).into()],
        ))
        .all(&self.db)
        .await
        .map_err(db_err)?;
        let now = Utc::now();
        Ok(rows
            .into_iter()
            .map(|r| Profile::from_row(r, now))
            .collect())
    }
}

// --- used by messaging ------------------------------------------------------------------------

/// The profile mobile, but only when it is verified and recent. Anything less and the caller must
/// use what the upstream record holds: an unproven number must not receive a patient's messages.
pub async fn trusted_mobile<C: ConnectionTrait>(
    conn: &C,
    person_id: Uuid,
) -> Result<Option<String>> {
    let row = conn
        .query_one_raw(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "SELECT mobile_e164 FROM patient_communication_profiles \
             WHERE person_id = $1 AND mobile_e164 IS NOT NULL AND mobile_verified_at IS NOT NULL \
               AND mobile_verified_at >= now() - ($2 || ' days')::interval",
            [person_id.into(), DEFAULT_STALE_DAYS.to_string().into()],
        ))
        .await
        .map_err(db_err)?;
    Ok(row.and_then(|r| r.try_get("", "mobile_e164").ok()))
}

/// The patient's preferred channel, if they set one.
pub async fn preferred_channel<C: ConnectionTrait>(
    conn: &C,
    person_id: Uuid,
) -> Result<Option<String>> {
    let row = conn
        .query_one_raw(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "SELECT preferred_channel FROM patient_communication_profiles WHERE person_id = $1",
            [person_id.into()],
        ))
        .await
        .map_err(db_err)?;
    Ok(row.and_then(|r| {
        r.try_get::<Option<String>>("", "preferred_channel")
            .ok()
            .flatten()
    }))
}

/// Has the patient said they can only be reached by letter (CAS-10)?
pub async fn letter_only<C: ConnectionTrait>(conn: &C, person_id: Uuid) -> Result<bool> {
    let row = conn
        .query_one_raw(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "SELECT 1 AS ok FROM patient_communication_profiles \
             WHERE person_id = $1 AND 'letter_only' = ANY(accessibility_needs)",
            [person_id.into()],
        ))
        .await
        .map_err(db_err)?;
    Ok(row.is_some())
}

/// The patient's preferred language, if they set one.
pub async fn preferred_language<C: ConnectionTrait>(
    conn: &C,
    person_id: Uuid,
) -> Result<Option<String>> {
    let row = conn
        .query_one_raw(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "SELECT preferred_language FROM patient_communication_profiles WHERE person_id = $1",
            [person_id.into()],
        ))
        .await
        .map_err(db_err)?;
    Ok(row.and_then(|r| {
        r.try_get::<Option<String>>("", "preferred_language")
            .ok()
            .flatten()
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_e164_accepts_international_forms_and_strips_punctuation() {
        assert_eq!(
            normalise_e164("+44 7700 900123", None).unwrap(),
            "+447700900123"
        );
        assert_eq!(
            normalise_e164("+1 (415) 555-2671", None).unwrap(),
            "+14155552671"
        );
        assert_eq!(
            normalise_e164("0044 7700 900123", None).unwrap(),
            "+447700900123",
            "00 means +"
        );
        assert_eq!(
            normalise_e164("+44.7700.900123", None).unwrap(),
            "+447700900123"
        );
    }

    #[test]
    fn test_e164_national_numbers_need_a_configured_country_code() {
        assert!(normalise_e164("07700 900123", None).is_err());
        assert_eq!(
            normalise_e164("07700 900123", Some("44")).unwrap(),
            "+447700900123"
        );
        assert!(
            normalise_e164("07700 900123", Some("+44")).is_err(),
            "the code is digits, without +"
        );
        assert!(normalise_e164("07700 900123", Some("")).is_err());
    }

    #[test]
    fn test_e164_refuses_what_is_not_a_number() {
        for bad in [
            "",
            "abc",
            "+",
            "+44 77a0 900123",
            "+0123456789",
            "+1234567",
            "+1234567890123456",
            "44 7700 900123",
            "+44 7700 900123 ext 5",
        ] {
            assert!(
                normalise_e164(bad, None).is_err(),
                "{bad:?} should be refused"
            );
        }
    }

    #[test]
    fn test_bcp47_drops_the_world_region_and_uppercases_others() {
        assert_eq!(bcp47("cy-001"), "cy");
        assert_eq!(bcp47("de-de"), "de-DE");
        assert_eq!(bcp47("zh-cn"), "zh-CN");
        assert_eq!(bcp47("en"), "en");
    }

    #[test]
    fn test_staleness_is_missing_unverified_or_old() {
        let now = Utc::now();
        assert!(is_stale(None, Some(now), now, 365), "no number at all");
        assert!(
            is_stale(Some("+447700900123"), None, now, 365),
            "never verified"
        );
        assert!(!is_stale(
            Some("+447700900123"),
            Some(now - Duration::days(364)),
            now,
            365
        ));
        assert!(is_stale(
            Some("+447700900123"),
            Some(now - Duration::days(366)),
            now,
            365
        ));
    }

    #[test]
    fn test_validate_refuses_unknown_channels_needs_and_bad_email() {
        let ok = ProfileInput {
            mobile: Some("+447700900123".into()),
            accessibility_needs: vec!["audio".into(), "audio".into(), "easy_read".into()],
            ..Default::default()
        };
        let v = validate(&ok, None).unwrap();
        assert_eq!(
            v.needs,
            vec!["audio", "easy_read"],
            "sorted and de-duplicated"
        );
        assert!(
            validate(
                &ProfileInput {
                    preferred_channel: Some("fax".into()),
                    ..Default::default()
                },
                None
            )
            .is_err()
        );
        assert!(
            validate(
                &ProfileInput {
                    accessibility_needs: vec!["telepathy".into()],
                    ..Default::default()
                },
                None
            )
            .is_err()
        );
        assert!(
            validate(
                &ProfileInput {
                    alternate_email: Some("not an email".into()),
                    ..Default::default()
                },
                None
            )
            .is_err()
        );
        assert!(
            validate(
                &ProfileInput {
                    alternate_name: Some("x".repeat(121)),
                    ..Default::default()
                },
                None
            )
            .is_err()
        );
    }

    #[test]
    fn test_fhir_fragment_carries_language_telecom_contact_and_needs() {
        let row = ProfileRow {
            person_id: Uuid::nil(),
            mobile_e164: Some("+447700900123".into()),
            mobile_verified_at: Some(Utc::now()),
            mobile_verification_method: Some("staff".into()),
            alternate_name: Some("Sam Jones".into()),
            alternate_relationship: Some("daughter".into()),
            alternate_phone_e164: Some("+447700900999".into()),
            alternate_email: None,
            alternate_is_carer: true,
            alternate_is_emergency: true,
            preferred_channel: Some("whatsapp".into()),
            preferred_language: Some("cy-001".into()),
            accessibility_needs: vec!["large_print".into()],
            updated_at: Utc::now(),
            updated_by: None,
        };
        let f = fhir_fragment(&row);
        assert_eq!(f["resourceType"], "Patient");
        assert_eq!(f["telecom"][0]["use"], "mobile");
        assert!(f["telecom"][0]["extension"][0]["valueDateTime"].is_string());
        assert_eq!(f["communication"][0]["language"]["coding"][0]["code"], "cy");
        assert_eq!(f["communication"][0]["preferred"], true);
        assert_eq!(f["contact"][0]["name"]["text"], "Sam Jones");
        assert_eq!(
            f["extension"][0]["valueCodeableConcept"]["coding"][0]["code"],
            "large_print"
        );
    }
}
