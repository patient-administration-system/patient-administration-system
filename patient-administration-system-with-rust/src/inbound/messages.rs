//! The words we text back (PILOT-4), in English and Welsh.
//!
//! **The Welsh here is a first draft and needs review by a fluent speaker before any pilot.** Every
//! message is short enough for one SMS segment where it can be. Times are shown in the clinic's time
//! zone (`PAS_TIMEZONE`, default Europe/London), because a patient reads "09:30" as local time.

use chrono::{DateTime, Datelike, Timelike, Utc};
use chrono_tz::Tz;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    En,
    Cy,
}

impl Lang {
    /// `cy`, `cy-GB`, `cy-001` are Welsh; anything else (or nothing) is English.
    pub fn from_preference(p: Option<&str>) -> Lang {
        match p {
            Some(l) if l.to_ascii_lowercase().starts_with("cy") => Lang::Cy,
            _ => Lang::En,
        }
    }
    pub fn code(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::Cy => "cy",
        }
    }
}

pub fn timezone() -> Tz {
    std::env::var("PAS_TIMEZONE")
        .ok()
        .and_then(|z| z.parse().ok())
        .unwrap_or(chrono_tz::Europe::London)
}

const EN_DAY: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
const EN_MONTH: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
const CY_DAY: [&str; 7] = ["Llun", "Maw", "Mer", "Iau", "Gwe", "Sad", "Sul"];
const CY_MONTH: [&str; 12] = [
    "Ion", "Chwef", "Maw", "Ebr", "Mai", "Meh", "Gorff", "Awst", "Medi", "Hyd", "Tach", "Rhag",
];

/// `Tue 3 Nov 09:30` / `Maw 3 Tach 09:30`, in the given zone.
pub fn when(at: DateTime<Utc>, lang: Lang, tz: Tz) -> String {
    let l = at.with_timezone(&tz);
    let (days, months) = match lang {
        Lang::En => (&EN_DAY, &EN_MONTH),
        Lang::Cy => (&CY_DAY, &CY_MONTH),
    };
    format!(
        "{} {} {} {:02}:{:02}",
        days[l.weekday().num_days_from_monday() as usize],
        l.day(),
        months[l.month0() as usize],
        l.hour(),
        l.minute()
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Confirmed,
    Cancelled,
    NeedsStaff,
    Unknown,
    Help,
    Stop,
    NoOpenReminder,
    Failed,
}

/// The reply for an outcome. `when` is the formatted appointment time where the message names one.
pub fn text(lang: Lang, kind: Kind, when: &str, help_line: Option<&str>) -> String {
    let base = match (lang, kind) {
        (Lang::En, Kind::Confirmed) => format!("Thank you, your appointment on {when} is confirmed."),
        (Lang::Cy, Kind::Confirmed) => format!("Diolch, mae eich apwyntiad ar {when} wedi'i gadarnhau."),
        (Lang::En, Kind::Cancelled) => format!("Your appointment on {when} is cancelled. Thank you for letting us know."),
        (Lang::Cy, Kind::Cancelled) => format!("Mae eich apwyntiad ar {when} wedi'i ganslo. Diolch am roi gwybod i ni."),
        (Lang::En, Kind::NeedsStaff) => format!("We will contact you to find a new time. Your appointment on {when} stays until then."),
        (Lang::Cy, Kind::NeedsStaff) => format!("Byddwn yn cysylltu â chi i ddod o hyd i amser newydd. Mae eich apwyntiad ar {when} yn aros tan hynny."),
        (Lang::En, Kind::Unknown) => "Sorry, we did not understand. Reply Y to confirm, N to cancel or R to change your appointment. Reply HELP for help.".into(),
        (Lang::Cy, Kind::Unknown) => "Mae'n ddrwg gennym, ni wnaethom ddeall. Atebwch IE i gadarnhau, NA i ganslo neu NEWID i newid eich apwyntiad. Atebwch CYMORTH am help.".into(),
        (Lang::En, Kind::Help) => "Reply Y to confirm, N to cancel, R to change, or STOP to stop messages.".into(),
        (Lang::Cy, Kind::Help) => "Atebwch IE i gadarnhau, NA i ganslo, NEWID i newid, neu STOPIO i atal negeseuon.".into(),
        (Lang::En, Kind::Stop) => "You will get no more text messages from us. Your appointments are not affected.".into(),
        (Lang::Cy, Kind::Stop) => "Ni chewch ragor o negeseuon testun gennym. Nid yw eich apwyntiadau'n cael eu heffeithio.".into(),
        (Lang::En, Kind::NoOpenReminder) => "We cannot find an appointment to update. Please call us.".into(),
        (Lang::Cy, Kind::NoOpenReminder) => "Ni allwn ddod o hyd i apwyntiad i'w ddiweddaru. Ffoniwch ni, os gwelwch yn dda.".into(),
        (Lang::En, Kind::Failed) => "Sorry, we could not update that. Please call us.".into(),
        (Lang::Cy, Kind::Failed) => "Mae'n ddrwg gennym, ni allem ddiweddaru hynny. Ffoniwch ni, os gwelwch yn dda.".into(),
    };
    match (help_line, kind) {
        (
            Some(h),
            Kind::Help | Kind::Unknown | Kind::NoOpenReminder | Kind::Failed | Kind::NeedsStaff,
        ) if !h.trim().is_empty() => {
            format!("{base} {}", h.trim())
        }
        _ => base,
    }
}

/// "Which appointment? Reply 1, 2 ..." with one line per candidate.
pub fn ask(lang: Lang, times: &[String]) -> String {
    let head = match lang {
        Lang::En => "Which appointment? Reply with a number:",
        Lang::Cy => "Pa apwyntiad? Atebwch gyda rhif:",
    };
    let list: Vec<String> = times
        .iter()
        .enumerate()
        .map(|(i, t)| format!("{} {}", i + 1, t))
        .collect();
    format!("{head} {}", list.join(", "))
}

/// Both languages, for a person we cannot identify (so not yet knowing theirs).
pub fn bilingual(kind: Kind) -> String {
    format!(
        "{} / {}",
        text(Lang::En, kind, "", None),
        text(Lang::Cy, kind, "", None)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn test_times_are_shown_in_the_clinic_zone_and_the_patients_language() {
        // 09:30 UTC in November is 09:30 in London (GMT); in July it is 10:30 (BST).
        let nov = Utc.with_ymd_and_hms(2026, 11, 3, 9, 30, 0).unwrap();
        let jul = Utc.with_ymd_and_hms(2026, 7, 7, 9, 30, 0).unwrap();
        let tz = chrono_tz::Europe::London;
        assert_eq!(when(nov, Lang::En, tz), "Tue 3 Nov 09:30");
        assert_eq!(when(nov, Lang::Cy, tz), "Maw 3 Tach 09:30");
        assert_eq!(when(jul, Lang::En, tz), "Tue 7 Jul 10:30");
    }

    #[test]
    fn test_language_comes_from_the_preference_with_english_as_the_default() {
        assert_eq!(Lang::from_preference(Some("cy")), Lang::Cy);
        assert_eq!(Lang::from_preference(Some("cy-GB")), Lang::Cy);
        assert_eq!(Lang::from_preference(Some("CY-001")), Lang::Cy);
        assert_eq!(Lang::from_preference(Some("en-001")), Lang::En);
        assert_eq!(Lang::from_preference(Some("ar")), Lang::En);
        assert_eq!(Lang::from_preference(None), Lang::En);
    }

    #[test]
    fn test_every_message_exists_in_both_languages_and_names_the_time_where_it_should() {
        for kind in [
            Kind::Confirmed,
            Kind::Cancelled,
            Kind::NeedsStaff,
            Kind::Unknown,
            Kind::Help,
            Kind::Stop,
            Kind::NoOpenReminder,
            Kind::Failed,
        ] {
            let en = text(Lang::En, kind, "Tue 3 Nov 09:30", None);
            let cy = text(Lang::Cy, kind, "Maw 3 Tach 09:30", None);
            assert!(!en.is_empty() && !cy.is_empty() && en != cy, "{kind:?}");
            let names_time = matches!(kind, Kind::Confirmed | Kind::Cancelled | Kind::NeedsStaff);
            assert_eq!(en.contains("Tue 3 Nov 09:30"), names_time, "{kind:?}");
            assert_eq!(cy.contains("Maw 3 Tach 09:30"), names_time, "{kind:?}");
        }
    }

    #[test]
    fn test_the_help_line_is_appended_only_where_a_person_can_help() {
        assert!(
            text(Lang::En, Kind::Unknown, "", Some("Call 01234 567890."))
                .ends_with("Call 01234 567890.")
        );
        assert!(
            !text(Lang::En, Kind::Confirmed, "x", Some("Call 01234 567890.")).contains("01234")
        );
        assert!(!text(Lang::En, Kind::Stop, "", Some("Call 01234 567890.")).contains("01234"));
    }

    #[test]
    fn test_the_question_lists_each_candidate_by_number() {
        assert_eq!(
            ask(
                Lang::En,
                &["Tue 3 Nov 09:30".into(), "Thu 5 Nov 14:00".into()]
            ),
            "Which appointment? Reply with a number: 1 Tue 3 Nov 09:30, 2 Thu 5 Nov 14:00"
        );
    }
}
