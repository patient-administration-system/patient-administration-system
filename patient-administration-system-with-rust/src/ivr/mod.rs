//! Voice reminders with keypad options (CAP-13).
//!
//! An outbound reminder call carries the same single-use token as a text or email link, and the telephony
//! carrier asks us what to say and what to do with each key press. The PAS does not place the call or run the
//! voice: it answers two questions, "what do I say" (`menu`) and "they pressed this" (`press`), over the same
//! safe machinery as the reply link, so everything the link guarantees holds here too.
//!
//! - **One choice per key**, in the patient's language (English or Welsh), the same options in both:
//!   1 confirm, 2 a different time, 3 cancel, 9 hear it again.
//! - **Cancel asks twice** (a voice call is easy to fumble): 3 then 1 to cancel, 2 to keep.
//! - **A different time offers real slots** (CAP-5): 1, 2, 3 choose one, 0 goes back; with none to offer a
//!   person phones.
//! - **Safe, because it is the link**: the token is single use, a held cancel on an urgent pathway is held, and
//!   an expired or used token says so and ends the call.
//! - **Stateless.** The carrier sends back the `state` it was last given (`menu`, `times`, `cancel_sure`), so
//!   a repeated or retried press changes nothing the token has not already settled.
//!
//! Not here: placing the call, text to speech, and the carrier contract (NOT-3).

use std::sync::Arc;

use serde::Serialize;

use crate::db::repositories::audit::UserContext;
use crate::inbound::messages::{self, Kind, Lang};
use crate::reply_links::{LinkAction, ReplyLinks};
use crate::{Error, Result};

#[derive(Debug, Clone, Serialize)]
pub struct IvrReply {
    /// What the call says next, in the patient's language.
    pub say: String,
    /// The keys that mean something now. Empty when the call is over.
    pub accepts: Vec<char>,
    /// Send this back with the next key press.
    pub state: &'static str,
    /// True when the call has done what it was for and should end.
    pub done: bool,
}

pub struct IvrService {
    links: Arc<ReplyLinks>,
}

impl IvrService {
    pub fn new(links: Arc<ReplyLinks>) -> Self {
        Self { links }
    }

    async fn lang_and_when(&self, token: &str) -> Result<(Lang, String, bool)> {
        let info = self.links.info(token).await?;
        let lang = Lang::from_preference(Some(info.language));
        Ok((lang, info.when, info.state == "open"))
    }

    /// The link is used, expired or invalid: say so and end the call. Nothing is changed.
    fn over(&self, lang: Lang) -> IvrReply {
        Self::finish(messages::text(lang, Kind::Failed, "", None))
    }

    /// What to say when the call connects.
    pub async fn menu(&self, token: &str) -> Result<IvrReply> {
        let (lang, when, open) = self.lang_and_when(token).await?;
        if !open {
            return Ok(self.over(lang));
        }
        Ok(IvrReply {
            say: messages::ivr_menu(lang, &when),
            accepts: vec!['1', '2', '3', '9'],
            state: "menu",
            done: false,
        })
    }

    fn finish(say: String) -> IvrReply {
        IvrReply {
            say,
            accepts: vec![],
            state: "done",
            done: true,
        }
    }

    /// The patient pressed `digit` in `state`.
    pub async fn press(
        &self,
        token: &str,
        state: &str,
        digit: char,
        ctx: &UserContext,
    ) -> Result<IvrReply> {
        let (lang, when, open) = self.lang_and_when(token).await?;
        if !open {
            return Ok(self.over(lang));
        }
        let menu = || IvrReply {
            say: messages::ivr_menu(lang, &when),
            accepts: vec!['1', '2', '3', '9'],
            state: "menu",
            done: false,
        };
        match (state, digit) {
            ("menu", '1') => {
                let o = self
                    .links
                    .act(token, LinkAction::Confirm, false, ctx)
                    .await?;
                Ok(Self::finish(o.message))
            }
            ("menu", '2') => self.times(token, lang, ctx).await,
            ("menu", '3') => Ok(IvrReply {
                say: messages::ivr_cancel_sure(lang),
                accepts: vec!['1', '2'],
                state: "cancel_sure",
                done: false,
            }),
            ("menu", '9') => Ok(menu()),
            ("times", '0') => Ok(menu()),
            ("times", d) if d.is_ascii_digit() => {
                let o = self
                    .links
                    .act(token, LinkAction::Reschedule, false, ctx)
                    .await?;
                let idx = d.to_digit(10).unwrap_or(0) as usize;
                match o.options.get(idx.wrapping_sub(1)) {
                    Some(opt) if idx >= 1 => {
                        let done = self.links.choose(token, opt.slot_id, ctx).await?;
                        Ok(Self::finish(done.message))
                    }
                    _ => Ok(self.retry_times(lang, &o.options)),
                }
            }
            ("cancel_sure", '1') => {
                let o = self.links.act(token, LinkAction::Cancel, true, ctx).await?;
                Ok(Self::finish(o.message))
            }
            ("cancel_sure", '2') => {
                let mut m = menu();
                m.say = format!("{} {}", messages::ivr_kept(lang), m.say);
                Ok(m)
            }
            ("cancel_sure", _) => Ok(IvrReply {
                say: format!(
                    "{} {}",
                    messages::ivr_retry(lang),
                    messages::ivr_cancel_sure(lang)
                ),
                accepts: vec!['1', '2'],
                state: "cancel_sure",
                done: false,
            }),
            ("menu", _) => {
                let mut m = menu();
                m.say = format!("{} {}", messages::ivr_retry(lang), m.say);
                Ok(m)
            }
            _ => Err(Error::validation(
                "state must be menu, times or cancel_sure",
            )),
        }
    }

    fn retry_times(&self, lang: Lang, options: &[crate::reply_links::LinkOption]) -> IvrReply {
        let whens: Vec<String> = options.iter().map(|o| o.when.clone()).collect();
        IvrReply {
            say: format!(
                "{} {}",
                messages::ivr_retry(lang),
                messages::ivr_times(lang, &whens)
            ),
            accepts: (1..=options.len())
                .filter_map(|n| char::from_digit(n as u32, 10))
                .chain(['0'])
                .collect(),
            state: "times",
            done: false,
        }
    }

    /// Key 2: offer the real alternative times, or hand over to a person when there are none.
    async fn times(&self, token: &str, lang: Lang, ctx: &UserContext) -> Result<IvrReply> {
        let o = self
            .links
            .act(token, LinkAction::Reschedule, false, ctx)
            .await?;
        if o.options.is_empty() {
            // Nothing to offer: `act` has put the patient on the staff call list and used the link.
            return Ok(Self::finish(o.message));
        }
        let whens: Vec<String> = o.options.iter().map(|x| x.when.clone()).collect();
        Ok(IvrReply {
            say: messages::ivr_times(lang, &whens),
            accepts: (1..=o.options.len())
                .filter_map(|n| char::from_digit(n as u32, 10))
                .chain(['0'])
                .collect(),
            state: "times",
            done: false,
        })
    }
}
