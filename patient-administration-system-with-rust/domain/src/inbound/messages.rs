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
    crate::config::env::text("PAS_TIMEZONE")
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
    Rebooked,
    HeldForCall,
    NeedsStaff,
    Unknown,
    Help,
    Stop,
    Started,
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
        (Lang::En, Kind::Rebooked) => format!("Thank you, your appointment has moved to {when}."),
        (Lang::Cy, Kind::Rebooked) => format!("Diolch, mae eich apwyntiad wedi symud i {when}."),
        (Lang::En, Kind::HeldForCall) => format!("Your appointment on {when} has not been cancelled. Because it is important, someone will phone you first."),
        (Lang::Cy, Kind::HeldForCall) => format!("Nid yw eich apwyntiad ar {when} wedi'i ganslo. Gan ei fod yn bwysig, bydd rhywun yn eich ffonio yn gyntaf."),
        (Lang::En, Kind::NeedsStaff) => format!("We will contact you to find a new time. Your appointment on {when} stays until then."),
        (Lang::Cy, Kind::NeedsStaff) => format!("Byddwn yn cysylltu â chi i ddod o hyd i amser newydd. Mae eich apwyntiad ar {when} yn aros tan hynny."),
        (Lang::En, Kind::Unknown) => "Sorry, we did not understand. Reply Y to confirm, N to cancel or R to change your appointment. Reply HELP for help.".into(),
        (Lang::Cy, Kind::Unknown) => "Mae'n ddrwg gennym, ni wnaethom ddeall. Atebwch IE i gadarnhau, NA i ganslo neu NEWID i newid eich apwyntiad. Atebwch CYMORTH am help.".into(),
        (Lang::En, Kind::Help) => "Reply Y to confirm, N to cancel, R to change, or STOP to stop messages.".into(),
        (Lang::Cy, Kind::Help) => "Atebwch IE i gadarnhau, NA i ganslo, NEWID i newid, neu STOPIO i atal negeseuon.".into(),
        (Lang::En, Kind::Stop) => "You will get no more text messages from us. Your appointments are not affected.".into(),
        (Lang::Cy, Kind::Stop) => "Ni chewch ragor o negeseuon testun gennym. Nid yw eich apwyntiadau'n cael eu heffeithio.".into(),
        (Lang::En, Kind::Started) => "You will get text messages from us again. Reply STOP at any time to stop them.".into(),
        (Lang::Cy, Kind::Started) => "Byddwch yn cael negeseuon testun gennym eto. Atebwch STOPIO unrhyw bryd i'w hatal.".into(),
        (Lang::En, Kind::NoOpenReminder) => "We cannot find an appointment to update. Please call us.".into(),
        (Lang::Cy, Kind::NoOpenReminder) => "Ni allwn ddod o hyd i apwyntiad i'w ddiweddaru. Ffoniwch ni, os gwelwch yn dda.".into(),
        (Lang::En, Kind::Failed) => "Sorry, we could not update that. Please call us.".into(),
        (Lang::Cy, Kind::Failed) => "Mae'n ddrwg gennym, ni allem ddiweddaru hynny. Ffoniwch ni, os gwelwch yn dda.".into(),
    };
    match (help_line, kind) {
        (
            Some(h),
            Kind::Help
            | Kind::Unknown
            | Kind::NoOpenReminder
            | Kind::Failed
            | Kind::NeedsStaff
            | Kind::HeldForCall,
        ) if !h.trim().is_empty() => {
            format!("{base} {}", h.trim())
        }
        _ => base,
    }
}

/// What to say to a patient who gave a reason for cancelling (CAS-2). Each points to real help and says plainly
/// that cancelling will not affect their care. First-draft wording: a person who knows the service should
/// review it (and the Welsh) before a pilot. `None` for a code with nothing useful to add.
pub fn signpost(lang: Lang, code: crate::models::appointment::BarrierCode) -> Option<String> {
    use crate::models::appointment::BarrierCode as B;
    let (en, cy) = match code {
        B::CannotTravel => (
            "Travel is a common problem. Tell us, and we may be able to arrange transport or offer a phone or video appointment. Cancelling will not affect your care.",
            "Mae teithio yn broblem gyffredin. Dywedwch wrthym, ac efallai y gallwn drefnu cludiant neu gynnig apwyntiad dros y ffôn neu fideo. Ni fydd canslo yn effeithio ar eich gofal.",
        ),
        B::Cost => (
            "If cost is a worry, help with travel costs may be available. Tell us and we will point you to it. Cancelling will not affect your care.",
            "Os yw cost yn bryder, efallai y bydd help gyda chostau teithio ar gael. Dywedwch wrthym a byddwn yn eich cyfeirio ato. Ni fydd canslo yn effeithio ar eich gofal.",
        ),
        B::Caring => (
            "If you care for someone, tell us and we will look for a time that works. Cancelling will not affect your care.",
            "Os ydych yn gofalu am rywun, dywedwch wrthym a byddwn yn chwilio am amser sy'n gweithio. Ni fydd canslo yn effeithio ar eich gofal.",
        ),
        B::Work => (
            "We can look for a time outside your working hours. Cancelling will not affect your care.",
            "Gallwn chwilio am amser y tu allan i'ch oriau gwaith. Ni fydd canslo yn effeithio ar eich gofal.",
        ),
        B::Unwell => (
            "We are sorry you are unwell. We will help you rebook when you are better. Cancelling will not affect your care.",
            "Mae'n ddrwg gennym eich bod yn sâl. Byddwn yn eich helpu i aildrefnu pan fyddwch yn well. Ni fydd canslo yn effeithio ar eich gofal.",
        ),
        B::Anxiety => (
            "It is common to feel worried about an appointment. Tell us what would help and we can talk it through. Cancelling will not affect your care.",
            "Mae'n gyffredin teimlo'n bryderus am apwyntiad. Dywedwch wrthym beth fyddai'n helpu a gallwn drafod. Ni fydd canslo yn effeithio ar eich gofal.",
        ),
        B::AccessNeed => (
            "Tell us what would help you to communicate and we will arrange it. Cancelling will not affect your care.",
            "Dywedwch wrthym beth fyddai'n eich helpu i gyfathrebu a byddwn yn ei drefnu. Ni fydd canslo yn effeithio ar eich gofal.",
        ),
        B::Other => (
            "Thank you for telling us. Cancelling will not affect your care.",
            "Diolch am ddweud wrthym. Ni fydd canslo yn effeithio ar eich gofal.",
        ),
        _ => return None,
    };
    Some(match lang {
        Lang::En => en.to_string(),
        Lang::Cy => cy.to_string(),
    })
}

/// What a voice call says (CAP-13): short, in the patient's language, one choice per key. Spoken text has no
/// links and no clinical detail.
pub fn ivr_menu(lang: Lang, when: &str) -> String {
    match lang {
        Lang::En => format!(
            "Your appointment is on {when}. Press 1 to confirm it. Press 2 to ask for a different time. Press 3 to cancel it. Press 9 to hear this again."
        ),
        Lang::Cy => format!(
            "Mae eich apwyntiad ar {when}. Pwyswch 1 i'w gadarnhau. Pwyswch 2 i ofyn am amser gwahanol. Pwyswch 3 i'w ganslo. Pwyswch 9 i glywed hyn eto."
        ),
    }
}

pub fn ivr_retry(lang: Lang) -> String {
    match lang {
        Lang::En => "Sorry, that was not one of the choices.".into(),
        Lang::Cy => "Mae'n ddrwg gennym, nid oedd hynny'n un o'r dewisiadau.".into(),
    }
}

/// "Press 1 for ..., 2 for ..., or 0 to go back" over the offered times.
pub fn ivr_times(lang: Lang, whens: &[String]) -> String {
    let list: Vec<String> = whens
        .iter()
        .enumerate()
        .map(|(i, w)| match lang {
            Lang::En => format!("Press {} for {}", i + 1, w),
            Lang::Cy => format!("Pwyswch {} am {}", i + 1, w),
        })
        .collect();
    match lang {
        Lang::En => format!("{}. Press 0 to go back.", list.join(". ")),
        Lang::Cy => format!("{}. Pwyswch 0 i fynd yn ôl.", list.join(". ")),
    }
}

pub fn ivr_cancel_sure(lang: Lang) -> String {
    match lang {
        Lang::En => format!(
            "{} Press 1 to cancel the appointment. Press 2 to keep it.",
            are_you_sure(lang)
        ),
        Lang::Cy => format!(
            "{} Pwyswch 1 i ganslo'r apwyntiad. Pwyswch 2 i'w gadw.",
            are_you_sure(lang)
        ),
    }
}

pub fn ivr_kept(lang: Lang) -> String {
    match lang {
        Lang::En => "Your appointment has not been changed.".into(),
        Lang::Cy => "Nid yw eich apwyntiad wedi'i newid.".into(),
    }
}

/// The times a "different time" reply is offered. The appointment stays until one is chosen.
pub fn rebook_options(lang: Lang, whens: &[String]) -> String {
    let (head, tail) = match lang {
        Lang::En => (
            "Other times we can offer. Reply with a number:",
            "Your appointment stays as it is unless you choose one.",
        ),
        Lang::Cy => (
            "Amserau eraill y gallwn eu cynnig. Atebwch gyda rhif:",
            "Mae eich apwyntiad yn aros fel y mae oni fyddwch yn dewis un.",
        ),
    };
    let list: Vec<String> = whens
        .iter()
        .enumerate()
        .map(|(i, t)| format!("{} {}", i + 1, t))
        .collect();
    format!("{head} {}. {tail}", list.join(", "))
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

/// The second step of Cancel.
pub fn are_you_sure(lang: Lang) -> String {
    match lang {
        Lang::En => "Are you sure you want to cancel this appointment? If you cancel, someone else may be given the slot.".into(),
        Lang::Cy => "Ydych chi'n siŵr eich bod am ganslo'r apwyntiad hwn? Os byddwch yn canslo, gall rhywun arall gael y slot.".into(),
    }
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
            Kind::Started,
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
