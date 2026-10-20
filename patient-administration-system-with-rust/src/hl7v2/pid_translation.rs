//! PID ↔ upstream `Person` translation.
//!
//! v0.43.0 had this conversion in the deleted `mapping.rs` (3068 lines)
//! talking to a local `Patient` model. Phase 3c reintroduces a minimal
//! PID → [`PersonCreate`] mapping that produces the upstream's JSON
//! shape directly — the PAS no longer holds a Patient type.
//!
//! ## PID fields honored
//!
//! - **PID-3** (Patient Identifier List, repeating): split on `~`. The
//!   first identifier becomes the upstream's MRN; additional identifiers
//!   are dropped on the floor at this minimum surface (Phase 3c2 will
//!   widen to handle the full repeating list incl. NHS / NIR / TSI /
//!   IHI / HCN system URIs).
//! - **PID-5** (Patient Name): `family^given^middle^suffix^prefix`.
//! - **PID-7** (Date of Birth): `YYYYMMDD` → `birth_date` (ISO 8601).
//! - **PID-8** (Sex): `M`/`F`/`O`/`U` → `male` / `female` / `other` /
//!   `unknown` (the upstream's lowercase convention).
//! - **PID-11** (Address): `line1^line2^city^state^zip^country` (single
//!   address; repetitions ignored at this surface).
//! - **PID-13** (Phone): `(area)number` → telecom entry with
//!   `system = "phone"`.
//!
//! Everything else (PID-2 external id, PID-6 mother maiden name, PID-9
//! patient alias, PID-10 race, PID-14 business phone, PID-16 marital
//! status, PID-22 ethnicity, PID-29 / PID-30 death info) is dropped at
//! this surface; Phase 3c2 can extend as use cases arise.

use serde_json::{Map, Value, json};

use crate::hl7v2::escape::unescape_value;
use crate::hl7v2::{Segment, SegmentRead};
use crate::upstream::PersonCreate;

/// Extract the PID-3.1 MRN value from a PID segment, if present.
/// Returns `None` when PID-3 is empty or has no first component.
pub fn extract_mrn_from_pid(pid: &Segment) -> Option<String> {
    let raw = pid.raw(3);
    if raw.is_empty() {
        return None;
    }
    // First repetition only (split on `~`).
    let first = raw.split('~').next().unwrap_or("");
    let mrn = first.split('^').next().unwrap_or("").trim();
    if mrn.is_empty() {
        None
    } else {
        Some(unescape_value(mrn))
    }
}

/// Convert a PID segment into the upstream's `PersonCreate` shape.
/// Missing fields are simply omitted from the resulting JSON map.
pub fn pid_to_person_create(pid: &Segment) -> PersonCreate {
    let mut fields: Map<String, Value> = Map::new();

    // PID-3: identifiers (first repetition only at this surface).
    if let Some(mrn) = extract_mrn_from_pid(pid) {
        let id_obj = json!({
            "use": "official",
            "type": "MRN",
            "value": mrn,
        });
        fields.insert("identifiers".to_string(), Value::Array(vec![id_obj]));
    }

    // PID-5: name.
    let name_raw = pid.raw(5);
    if !name_raw.is_empty() {
        let mut parts = name_raw.split('^');
        let family = unescape_value(parts.next().unwrap_or("").trim());
        let given_main = unescape_value(parts.next().unwrap_or("").trim());
        let middle = unescape_value(parts.next().unwrap_or("").trim());
        let suffix = unescape_value(parts.next().unwrap_or("").trim());
        let prefix = unescape_value(parts.next().unwrap_or("").trim());
        let mut given: Vec<Value> = Vec::new();
        if !given_main.is_empty() {
            given.push(Value::String(given_main));
        }
        if !middle.is_empty() {
            given.push(Value::String(middle));
        }
        let mut name = Map::new();
        if !family.is_empty() {
            name.insert("family".to_string(), Value::String(family));
        }
        if !given.is_empty() {
            name.insert("given".to_string(), Value::Array(given));
        }
        if !prefix.is_empty() {
            name.insert(
                "prefix".to_string(),
                Value::Array(vec![Value::String(prefix)]),
            );
        }
        if !suffix.is_empty() {
            name.insert(
                "suffix".to_string(),
                Value::Array(vec![Value::String(suffix)]),
            );
        }
        if !name.is_empty() {
            fields.insert("name".to_string(), Value::Object(name));
        }
    }

    // PID-7: birth_date (YYYYMMDD → YYYY-MM-DD).
    let bd_text = pid.raw(7);
    let bd = bd_text.trim();
    if bd.len() >= 8 && bd.is_ascii() {
        let (y, m, d) = (&bd[0..4], &bd[4..6], &bd[6..8]);
        if y.chars().all(|c| c.is_ascii_digit())
            && m.chars().all(|c| c.is_ascii_digit())
            && d.chars().all(|c| c.is_ascii_digit())
        {
            fields.insert(
                "birth_date".to_string(),
                Value::String(format!("{y}-{m}-{d}")),
            );
        }
    }

    // PID-8: sex.
    let sex_text = pid.raw(8);
    let sex = sex_text.trim();
    if !sex.is_empty() {
        let gender = match sex.to_ascii_uppercase().as_str() {
            "M" => "male",
            "F" => "female",
            "O" => "other",
            "U" | "" => "unknown",
            _ => "unknown",
        };
        fields.insert("gender".to_string(), Value::String(gender.to_string()));
    }

    // PID-11: address (single).
    let addr_raw = pid.raw(11);
    if !addr_raw.is_empty() {
        let mut parts = addr_raw.split('^');
        let line1 = unescape_value(parts.next().unwrap_or("").trim());
        let line2 = unescape_value(parts.next().unwrap_or("").trim());
        let city = unescape_value(parts.next().unwrap_or("").trim());
        let state = unescape_value(parts.next().unwrap_or("").trim());
        let postal = unescape_value(parts.next().unwrap_or("").trim());
        let country = unescape_value(parts.next().unwrap_or("").trim());
        let mut addr = Map::new();
        if !line1.is_empty() {
            addr.insert("line1".to_string(), Value::String(line1));
        }
        if !line2.is_empty() {
            addr.insert("line2".to_string(), Value::String(line2));
        }
        if !city.is_empty() {
            addr.insert("city".to_string(), Value::String(city));
        }
        if !state.is_empty() {
            addr.insert("state".to_string(), Value::String(state));
        }
        if !postal.is_empty() {
            addr.insert("postal_code".to_string(), Value::String(postal));
        }
        if !country.is_empty() {
            addr.insert("country".to_string(), Value::String(country));
        }
        if !addr.is_empty() {
            fields.insert(
                "addresses".to_string(),
                Value::Array(vec![Value::Object(addr)]),
            );
        }
    }

    // PID-13: telecom (single phone, first repetition).
    let phone_raw_text = pid.raw(13);
    let phone_raw = phone_raw_text.split('~').next().unwrap_or("").trim();
    if !phone_raw.is_empty() {
        let value = unescape_value(phone_raw);
        let cp = json!({ "system": "phone", "value": value });
        fields.insert("telecom".to_string(), Value::Array(vec![cp]));
    }

    PersonCreate { fields }
}

/// Extract the source MRN from an `MRG` segment (used by ADT^A40 merge).
/// MRG-1 has the same `value^check^scheme^facility^id_type` shape as
/// PID-3; we only care about the first component of the first
/// repetition.
pub fn extract_mrn_from_mrg(mrg: &Segment) -> Option<String> {
    let raw = mrg.raw(1);
    if raw.is_empty() {
        return None;
    }
    let first = raw.split('~').next().unwrap_or("");
    let mrn = first.split('^').next().unwrap_or("").trim();
    if mrn.is_empty() {
        None
    } else {
        Some(unescape_value(mrn))
    }
}

// ─── Segment parsers used by DFT / SIU / MFN handlers ───────────────────

/// Parsed FT1 (Financial Transaction) segment fields the PAS cares about.
/// FT1-4 posted_at, FT1-6 transaction_type, FT1-7.1 code, FT1-8 description,
/// FT1-11.1 amount, FT1-11.2 currency. Returns a typed value so the DFT^P03
/// handler can iterate and insert charges without re-parsing.
#[derive(Debug, Clone)]
pub struct Ft1 {
    pub set_id: u32,
    pub posted_at: Option<chrono::DateTime<chrono::Utc>>,
    pub transaction_type: String,
    pub code: String,
    pub description: String,
    pub amount: String,
    pub currency: String,
}

/// Parse one FT1 segment. The 1-based `set_id` is the FT1-1 set-id field
/// (mostly for diagnostics — "FT1[2]-11.1 ..."). Returns
/// `Err(Error::Validation)` when a required field is missing.
pub fn parse_ft1(ft1: &Segment) -> crate::Result<Ft1> {
    let set_id_text = ft1.raw(1);
    let set_id = set_id_text.trim().parse::<u32>().unwrap_or(0);
    let posted_at = parse_hl7_datetime(ft1.raw(4).trim());
    let transaction_type_text = ft1.raw(6);
    let transaction_type = transaction_type_text.trim().to_string();
    // FT1-7 is composite; 7.1 is the code.
    let code = ft1
        .raw(7)
        .split('^')
        .next()
        .unwrap_or("")
        .trim()
        .to_string();
    if code.is_empty() {
        return Err(crate::Error::validation(format!(
            "FT1[{set_id}]-7.1 (transaction code) is required"
        )));
    }
    let description_text = ft1.raw(8);
    let description = description_text.trim().to_string();
    if description.is_empty() {
        return Err(crate::Error::validation(format!(
            "FT1[{set_id}]-8 (description) is required"
        )));
    }
    let amount_field = ft1.raw(11);
    let mut amt_parts = amount_field.split('^');
    let amount = amt_parts.next().unwrap_or("").trim().to_string();
    let currency = amt_parts.next().unwrap_or("").trim().to_string();
    if amount.is_empty() {
        return Err(crate::Error::validation(format!(
            "FT1[{set_id}]-11.1 (amount) is required"
        )));
    }
    if currency.is_empty() {
        return Err(crate::Error::validation(format!(
            "FT1[{set_id}]-11.2 (currency) is required"
        )));
    }
    Ok(Ft1 {
        set_id,
        posted_at,
        transaction_type,
        code,
        description,
        amount,
        currency,
    })
}

/// Parse `YYYYMMDDHHMMSS` (or shorter `YYYYMMDDHHMM` / `YYYYMMDD`) into
/// UTC. Returns `None` on malformed input.
pub fn parse_hl7_datetime(raw: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    use chrono::{NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc};
    let s = raw.split(['+', '-']).next().unwrap_or("");
    if s.is_empty() || !s.is_ascii() {
        return None;
    }
    let (date, time) = match s.len() {
        14 => {
            let d = NaiveDate::from_ymd_opt(
                s[0..4].parse().ok()?,
                s[4..6].parse().ok()?,
                s[6..8].parse().ok()?,
            )?;
            let t = NaiveTime::from_hms_opt(
                s[8..10].parse().ok()?,
                s[10..12].parse().ok()?,
                s[12..14].parse().ok()?,
            )?;
            (d, t)
        }
        12 => {
            let d = NaiveDate::from_ymd_opt(
                s[0..4].parse().ok()?,
                s[4..6].parse().ok()?,
                s[6..8].parse().ok()?,
            )?;
            let t = NaiveTime::from_hms_opt(s[8..10].parse().ok()?, s[10..12].parse().ok()?, 0)?;
            (d, t)
        }
        8 => {
            let d = NaiveDate::from_ymd_opt(
                s[0..4].parse().ok()?,
                s[4..6].parse().ok()?,
                s[6..8].parse().ok()?,
            )?;
            (d, NaiveTime::from_hms_opt(0, 0, 0)?)
        }
        _ => return None,
    };
    let ndt = NaiveDateTime::new(date, time);
    Some(Utc.from_utc_datetime(&ndt))
}

/// Parsed SCH (Schedule Activity Information) segment fields the PAS
/// SIU handlers care about. SCH-1 placer id (opaque, kept for echo);
/// SCH-2 filler id (PAS UUID); SCH-7 reason; SCH-9/10 duration; SCH-11
/// start datetime.
#[derive(Debug, Clone)]
pub struct Sch {
    pub placer_id: Option<String>,
    pub filler_id: Option<uuid::Uuid>,
    pub reason: Option<String>,
    pub duration_minutes: Option<u32>,
    pub start: Option<chrono::DateTime<chrono::Utc>>,
}

pub fn parse_sch(sch: &Segment) -> Sch {
    let placer = {
        let raw_text = sch.raw(1);
        let raw = raw_text.trim();
        if raw.is_empty() {
            None
        } else {
            Some(unescape_value(raw))
        }
    };
    let filler = {
        let raw_text = sch.raw(2);
        let raw = raw_text.trim();
        if raw.is_empty() {
            None
        } else {
            uuid::Uuid::parse_str(raw).ok()
        }
    };
    let reason = {
        let raw_text = sch.raw(7);
        let raw = raw_text.trim();
        if raw.is_empty() {
            None
        } else {
            Some(unescape_value(raw))
        }
    };
    let duration_minutes = {
        let value_text = sch.raw(9);
        let value: Option<u32> = value_text.trim().parse().ok();
        let units_text = sch.raw(10);
        let units = units_text.trim();
        value.map(|v| match units.to_ascii_lowercase().as_str() {
            "h" | "hr" | "hours" => v * 60,
            "d" | "day" | "days" => v * 60 * 24,
            _ => v, // default minutes
        })
    };
    let start = parse_hl7_datetime(sch.raw(11).trim());
    Sch {
        placer_id: placer,
        filler_id: filler,
        reason,
        duration_minutes,
        start,
    }
}

/// Parsed MFE+STF pair from MFN^M02. Used to add/update/soft-delete a
/// worker. `event_code` is `MAD` / `MUP` / `MDL`. `staff_id` is the
/// EMR's identifier — MFE-4 with fallback to STF-1. The worker's family
/// and given name come from STF-3 (XPN), gender from STF-5, birth_date
/// from STF-6, active from STF-7.
#[derive(Debug, Clone)]
pub struct StaffItem {
    pub event_code: String,
    pub staff_id: String,
    pub family: String,
    pub given: String,
    pub middle: Option<String>,
    pub gender: Option<String>,
    pub birth_date: Option<String>,
    pub active: bool,
}

pub fn parse_mfe_stf(mfe: &Segment, stf: &Segment) -> crate::Result<StaffItem> {
    let event_code_text = mfe.raw(1);
    let event_code = event_code_text.trim().to_ascii_uppercase();
    if !matches!(event_code.as_str(), "MAD" | "MUP" | "MDL") {
        return Err(crate::Error::validation(format!(
            "MFE-1 must be MAD/MUP/MDL, got {event_code:?}"
        )));
    }
    let staff_id = {
        let raw_text = mfe.raw(4);
        let raw = raw_text.trim();
        if !raw.is_empty() {
            unescape_value(raw)
        } else {
            let alt_text = stf.raw(1);
            let alt = alt_text.trim();
            if alt.is_empty() {
                return Err(crate::Error::validation(
                    "MFE-4 or STF-1 (staff id) is required",
                ));
            }
            unescape_value(alt)
        }
    };
    let name_raw = stf.raw(3);
    let mut parts = name_raw.split('^');
    let family = unescape_value(parts.next().unwrap_or("").trim());
    let given = unescape_value(parts.next().unwrap_or("").trim());
    let middle_raw = unescape_value(parts.next().unwrap_or("").trim());
    let middle = if middle_raw.is_empty() {
        None
    } else {
        Some(middle_raw)
    };
    let gender = {
        let raw_text = stf.raw(5);
        let raw = raw_text.trim().to_ascii_uppercase();
        match raw.as_str() {
            "M" => Some("male".to_string()),
            "F" => Some("female".to_string()),
            "O" => Some("other".to_string()),
            "U" => Some("unknown".to_string()),
            "" => None,
            _ => Some("unknown".to_string()),
        }
    };
    let birth_date = {
        let raw_text = stf.raw(6);
        let raw = raw_text.trim();
        if raw.len() >= 8 && raw.is_ascii() {
            let (y, m, d) = (&raw[0..4], &raw[4..6], &raw[6..8]);
            if y.chars().all(|c| c.is_ascii_digit())
                && m.chars().all(|c| c.is_ascii_digit())
                && d.chars().all(|c| c.is_ascii_digit())
            {
                Some(format!("{y}-{m}-{d}"))
            } else {
                None
            }
        } else {
            None
        }
    };
    let active = matches!(
        stf.raw(7).trim().to_ascii_uppercase().as_str(),
        "I" | "INACTIVE"
    )
    .not();
    Ok(StaffItem {
        event_code,
        staff_id,
        family,
        given,
        middle,
        gender,
        birth_date,
        active,
    })
}

// Tiny extension trait so the active flag default-A can be expressed as
// `!matches!(…, "I" | "INACTIVE")`. Avoids pulling a whole `Not` impl.
trait NotExt {
    fn not(self) -> bool;
}
impl NotExt for bool {
    fn not(self) -> bool {
        !self
    }
}

/// Parsed MFE+LOC pair from MFN^M05 (location/bed master file).
/// `event_code` is MAD/MUP/MDL. `bed_code` is the operator key from
/// LOC-1.3 (the third PL component). `room_code` is LOC-1.2 (second PL
/// component) when present — MAD requires it so the new bed can be
/// attached to a room. `bed_name` is LOC-2.
#[derive(Debug, Clone)]
pub struct LocationItem {
    pub event_code: String,
    pub bed_code: String,
    pub room_code: Option<String>,
    pub bed_name: Option<String>,
}

pub fn parse_mfe_loc(mfe: &Segment, loc: &Segment) -> crate::Result<LocationItem> {
    let event_code_text = mfe.raw(1);
    let event_code = event_code_text.trim().to_ascii_uppercase();
    if !matches!(event_code.as_str(), "MAD" | "MUP" | "MDL") {
        return Err(crate::Error::validation(format!(
            "MFE-1 must be MAD/MUP/MDL, got {event_code:?}"
        )));
    }
    let raw = loc.raw(1);
    let mut parts = raw.split('^');
    let _ward = parts.next().unwrap_or("");
    let room_raw = parts.next().unwrap_or("").trim();
    let bed_raw = parts.next().unwrap_or("").trim();
    if bed_raw.is_empty() {
        return Err(crate::Error::validation("LOC-1.3 (bed code) is required"));
    }
    let bed_code = unescape_value(bed_raw);
    let room_code = if room_raw.is_empty() {
        None
    } else {
        Some(unescape_value(room_raw))
    };
    let bed_name = {
        let raw_text = loc.raw(2);
        let raw = raw_text.trim();
        if raw.is_empty() {
            None
        } else {
            Some(unescape_value(raw))
        }
    };
    Ok(LocationItem {
        event_code,
        bed_code,
        room_code,
        bed_name,
    })
}

/// Read PV1-2 (Patient Class) — `E` = Emergency, anything else (or
/// missing) → Outpatient. Used by ADT^A04 register-outpatient to
/// pick the encounter class.
pub fn pv1_class_or_outpatient(pv1: &Segment) -> &'static str {
    let raw_text = pv1.raw(2);
    let raw = raw_text.trim();
    if raw.eq_ignore_ascii_case("E") {
        "Emergency"
    } else {
        "Outpatient"
    }
}

/// Extract the PV1-3.3 bed code (3rd component of PV1-3, the
/// Assigned Patient Location field). Returns `None` when missing.
pub fn extract_bed_code_from_pv1(pv1: &Segment) -> Option<String> {
    let raw = pv1.raw(3);
    if raw.is_empty() {
        return None;
    }
    let bed = raw.split('^').nth(2)?.trim();
    if bed.is_empty() {
        None
    } else {
        Some(unescape_value(bed))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(name: &str, fields: &[&str]) -> Segment {
        crate::hl7v2::segment(name, fields.iter().map(|f| (*f).to_string()).collect())
    }

    #[test]
    fn test_extract_mrn_first_rep_first_component() {
        let pid = seg("PID", &["1", "", "MRN-001^^^FAC^MR", ""]);
        assert_eq!(extract_mrn_from_pid(&pid).as_deref(), Some("MRN-001"));
    }

    #[test]
    fn test_extract_mrn_empty_field_returns_none() {
        let pid = seg("PID", &["1", "", "", ""]);
        assert!(extract_mrn_from_pid(&pid).is_none());
    }

    #[test]
    fn test_extract_mrn_uses_first_repetition() {
        let pid = seg(
            "PID",
            &["1", "", "FIRST-MRN^^^FAC^MR~SECOND-MRN^^^FAC2^MR", ""],
        );
        assert_eq!(extract_mrn_from_pid(&pid).as_deref(), Some("FIRST-MRN"));
    }

    #[test]
    fn test_pid_to_person_full_fields() {
        let pid = seg(
            "PID",
            &[
                "1",
                "",
                "MRN-001^^^FAC^MR",
                "",
                "Doe^Jane^Marie",
                "",
                "19900115",
                "F",
                "",
                "",
                "123 Elm^Apt 4^Springfield^IL^62701^US",
                "",
                "(555)555-0100",
            ],
        );
        let pc = pid_to_person_create(&pid);
        assert_eq!(
            pc.fields["name"]["family"].as_str(),
            Some("Doe"),
            "got {:?}",
            pc.fields
        );
        assert_eq!(pc.fields["name"]["given"][0].as_str(), Some("Jane"));
        assert_eq!(pc.fields["name"]["given"][1].as_str(), Some("Marie"));
        assert_eq!(pc.fields["birth_date"].as_str(), Some("1990-01-15"));
        assert_eq!(pc.fields["gender"].as_str(), Some("female"));
        assert_eq!(
            pc.fields["addresses"][0]["postal_code"].as_str(),
            Some("62701")
        );
        assert_eq!(
            pc.fields["telecom"][0]["value"].as_str(),
            Some("(555)555-0100")
        );
        assert_eq!(
            pc.fields["identifiers"][0]["value"].as_str(),
            Some("MRN-001")
        );
    }

    #[test]
    fn test_pid_to_person_missing_fields_omits_them() {
        let pid = seg("PID", &["1", "", "MRN-002^^^FAC^MR", "", "Doe^John"]);
        let pc = pid_to_person_create(&pid);
        assert!(pc.fields.contains_key("name"));
        assert!(pc.fields.contains_key("identifiers"));
        assert!(!pc.fields.contains_key("birth_date"));
        assert!(!pc.fields.contains_key("gender"));
        assert!(!pc.fields.contains_key("addresses"));
        assert!(!pc.fields.contains_key("telecom"));
    }

    #[test]
    fn test_gender_unknown_default() {
        let pid = seg(
            "PID",
            &["1", "", "MRN^^^FAC^MR", "", "Doe^Jane", "", "", "X"],
        );
        let pc = pid_to_person_create(&pid);
        assert_eq!(pc.fields["gender"].as_str(), Some("unknown"));
    }

    #[test]
    fn test_extract_bed_code_from_pv1() {
        let pv1 = seg("PV1", &["1", "I", "WARD1^ROOM1^BED1"]);
        assert_eq!(extract_bed_code_from_pv1(&pv1).as_deref(), Some("BED1"));
    }

    #[test]
    fn test_extract_bed_code_missing_third_component() {
        let pv1 = seg("PV1", &["1", "I", "WARD1^ROOM1"]);
        assert!(extract_bed_code_from_pv1(&pv1).is_none());
    }

    #[test]
    fn test_extract_bed_code_empty_field() {
        let pv1 = seg("PV1", &["1", "I", ""]);
        assert!(extract_bed_code_from_pv1(&pv1).is_none());
    }

    #[test]
    fn test_extract_mrn_from_mrg() {
        let mrg = seg("MRG", &["MRN-OLD^^^FAC^MR"]);
        assert_eq!(extract_mrn_from_mrg(&mrg).as_deref(), Some("MRN-OLD"));
    }

    #[test]
    fn test_extract_mrn_from_mrg_empty() {
        let mrg = seg("MRG", &[""]);
        assert!(extract_mrn_from_mrg(&mrg).is_none());
    }

    #[test]
    fn test_pv1_class_emergency() {
        let pv1 = seg("PV1", &["1", "E"]);
        assert_eq!(pv1_class_or_outpatient(&pv1), "Emergency");
    }

    #[test]
    fn test_pv1_class_default_outpatient() {
        let pv1 = seg("PV1", &["1", "O"]);
        assert_eq!(pv1_class_or_outpatient(&pv1), "Outpatient");
    }

    #[test]
    fn test_pv1_class_missing_defaults_outpatient() {
        let pv1 = seg("PV1", &["1"]);
        assert_eq!(pv1_class_or_outpatient(&pv1), "Outpatient");
    }

    #[test]
    fn test_multibyte_dates_are_not_dates_and_do_not_panic() {
        // Found by the chaos monkey: byte slicing a date field that holds a multi-byte character.
        assert!(parse_hl7_datetime("💥🔥ñ").is_none());
        assert!(parse_hl7_datetime("2026💥1020").is_none());
        let pid = seg(
            "PID",
            &["1", "", "MRN1", "", "Name^Given", "", "💥🔥ñ💥🔥ñ"],
        );
        let _ = pid_to_person_create(&pid);
    }
}
