//! Template variants by language (CAP-9, PILOT-5).
//!
//! A *group* is every template with the same `name`; each member has a `locale`. A group may be active
//! only when it has an active variant for **every required locale** (`PAS_REQUIRED_LOCALES`, default
//! `en-001,cy-001`), so a patient is never sent a reminder in a language we have not written. The
//! reminder picks the variant for the patient's preferred language and falls back to the first required
//! locale (English) when the patient has no preference or it is a language we do not offer.

use sea_orm::ConnectionTrait;

use crate::config::env;
use crate::db::repositories::letter::LetterRepository;
use crate::models::communication::LetterTemplate;
use crate::{Error, Result};

pub const DEFAULT_REQUIRED_LOCALES: &str = "en-001,cy-001";

/// The locales every active template group must have.
pub fn required_locales() -> Vec<String> {
    let raw = env::text("PAS_REQUIRED_LOCALES").unwrap_or_default();
    let list: Vec<String> = if raw.trim().is_empty() {
        DEFAULT_REQUIRED_LOCALES
    } else {
        raw.as_str()
    }
    .split(',')
    .map(|s| s.trim().to_string())
    .filter(|s| !s.is_empty())
    .collect();
    list
}

/// A locale code such as `en-001`, `cy-001`, `cy`, `ar-EG`.
pub fn validate_locale(l: &str) -> Result<()> {
    let ok = !l.is_empty()
        && l.len() <= 16
        && l.split('-').enumerate().all(|(i, part)| {
            !part.is_empty()
                && part.chars().all(|c| c.is_ascii_alphanumeric())
                && (i > 0
                    || part.chars().all(|c| c.is_ascii_lowercase())
                        && (2..=3).contains(&part.len()))
        });
    if ok {
        Ok(())
    } else {
        Err(Error::validation(format!(
            "locale {l:?} is not a language code like en-001 or cy-001"
        )))
    }
}

/// The language part, lower case: `cy-GB` and `CY` are both `cy`.
pub fn primary(locale: &str) -> String {
    locale
        .split(['-', '_'])
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
}

/// Required locales not present among `present` (compared by exact code).
pub fn missing(required: &[String], present: &[String]) -> Vec<String> {
    required
        .iter()
        .filter(|r| !present.iter().any(|p| p == *r))
        .cloned()
        .collect()
}

/// The variant for a patient: their language if the group has it, else the first required locale's
/// variant, else the first variant at all.
pub fn choose<'a>(
    variants: &'a [LetterTemplate],
    preferred: Option<&str>,
    required: &[String],
) -> Option<&'a LetterTemplate> {
    if let Some(p) = preferred.map(primary).filter(|p| !p.is_empty())
        && let Some(v) = variants.iter().find(|v| primary(&v.locale) == p)
    {
        return Some(v);
    }
    required
        .first()
        .and_then(|d| variants.iter().find(|v| primary(&v.locale) == primary(d)))
        .or_else(|| variants.first())
}

/// A group is acceptable when it has no active variants at all, or every required locale is covered.
pub fn check_complete(name: &str, active_locales: &[String], required: &[String]) -> Result<()> {
    if active_locales.is_empty() {
        return Ok(());
    }
    let gaps = missing(required, active_locales);
    if gaps.is_empty() {
        Ok(())
    } else {
        Err(Error::validation(format!(
            "template {name:?} would be active without a {} version; every active template needs {} \
             (create them together with POST /api/letter-template-groups, or keep it inactive until the set is complete)",
            gaps.join(" and "),
            required.join(" and ")
        )))
    }
}

/// The locales of the active variants of a group, optionally treating one template as changed.
pub async fn active_locales_after<C: ConnectionTrait>(
    conn: &C,
    name: &str,
    change: Option<&LetterTemplate>,
) -> Result<Vec<String>> {
    let mut rows = LetterRepository::list_active_by_name(conn, name).await?;
    if let Some(c) = change {
        rows.retain(|r| r.id != c.id);
        if c.active && c.name == name {
            rows.push(c.clone());
        }
    }
    Ok(rows.into_iter().map(|r| r.locale).collect())
}

/// What a body would cost as a text, rendered with typical values. `None` when it will not render with
/// the sample (for example it needs a variable only a caller supplies).
pub fn sample_sms(
    body_tera: &str,
    locale: &str,
) -> Option<crate::communication::sms_length::SmsLength> {
    use crate::inbound::messages::{Lang, when};
    let lang = Lang::from_preference(Some(locale));
    let at = chrono::DateTime::parse_from_rfc3339("2026-11-03T09:30:00Z")
        .ok()?
        .with_timezone(&chrono::Utc);
    let mut ctx = tera::Context::new();
    ctx.insert(
        "patient",
        &serde_json::json!({ "name": { "given": "Ffion", "family": "Williams" } }),
    );
    ctx.insert(
        "appointment",
        &serde_json::json!({
            "when": when(at, lang, chrono_tz::Europe::London),
            "start": at.to_rfc3339(), "end": at.to_rfc3339(), "reason": "Knee review", "id": "00000000-0000-0000-0000-000000000000"
        }),
    );
    ctx.insert("portal_url", "https://portal.example.nhs.wales/my/messages");
    let rendered = tera::Tera::one_off(body_tera, &ctx, false).ok()?;
    Some(crate::communication::sms_length::analyze(&rendered))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(locale: &str) -> LetterTemplate {
        let mut t = LetterTemplate::new("n".into(), "s".into(), "b".into());
        t.locale = locale.into();
        t
    }
    fn req() -> Vec<String> {
        vec!["en-001".into(), "cy-001".into()]
    }

    #[test]
    fn test_locale_codes() {
        for ok in ["en", "en-001", "cy-001", "cy", "ar-EG", "fil", "zh-Hans-CN"] {
            assert!(validate_locale(ok).is_ok(), "{ok}");
        }
        for bad in [
            "",
            "EN",
            "english",
            "en_001",
            "en-",
            "-en",
            "e",
            "en-0 1",
            "en-001-aaaaaaaaaaaaaaaa",
        ] {
            assert!(validate_locale(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn test_the_variant_follows_the_patients_language_with_english_as_the_fallback() {
        let v = vec![t("en-001"), t("cy-001")];
        assert_eq!(choose(&v, Some("cy-001"), &req()).unwrap().locale, "cy-001");
        assert_eq!(choose(&v, Some("cy"), &req()).unwrap().locale, "cy-001");
        assert_eq!(choose(&v, Some("CY-GB"), &req()).unwrap().locale, "cy-001");
        assert_eq!(choose(&v, Some("en-001"), &req()).unwrap().locale, "en-001");
        assert_eq!(
            choose(&v, Some("ar"), &req()).unwrap().locale,
            "en-001",
            "a language we do not offer"
        );
        assert_eq!(choose(&v, None, &req()).unwrap().locale, "en-001");
        assert_eq!(choose(&v, Some(""), &req()).unwrap().locale, "en-001");
        assert!(choose(&[], Some("cy"), &req()).is_none());
        let only_cy = vec![t("cy-001")];
        assert_eq!(
            choose(&only_cy, Some("en"), &req()).unwrap().locale,
            "cy-001",
            "something rather than nothing"
        );
    }

    #[test]
    fn test_a_group_is_complete_when_every_required_locale_is_active() {
        let r = req();
        assert!(
            check_complete("n", &[], &r).is_ok(),
            "an inactive group is fine"
        );
        assert!(check_complete("n", &["en-001".into(), "cy-001".into()], &r).is_ok());
        let e = check_complete("n", &["en-001".into()], &r)
            .unwrap_err()
            .to_string();
        assert!(
            e.contains("cy-001") && e.contains("letter-template-groups"),
            "{e}"
        );
        assert!(check_complete("n", &["cy-001".into()], &r).is_err());
        assert_eq!(missing(&r, &["en-001".into()]), vec!["cy-001".to_string()]);
    }

    #[test]
    fn test_required_locales_default_to_english_and_welsh() {
        // (The environment is not set in unit tests.)
        if std::env::var("PAS_REQUIRED_LOCALES").is_err() {
            assert_eq!(required_locales(), req());
        }
    }
}
