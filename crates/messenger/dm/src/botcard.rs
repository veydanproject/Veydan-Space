// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The cards of the bot of Veydan Cloud (messenger-wire.md §3, "Карточки
//! бота"; cloud-recovery.md §10). The bot writes ordinary messages from a
//! key pinned in the build (`DmService::with_bot`); an envelope of that key
//! that is not a message of a chat is a card: a row of the chat
//! (`CT_BOT_CARD`) the UI draws by its type, and the event
//! [`UI_EVENT_BOT_CARD`] for the module `cloud`.
//!
//! The words of a card are the app's own, fixed by its type. Of what the
//! bot wrote only the values the contract names reach the UI, and only as
//! what they are: a number, or a short word without a space, a dot or a
//! slash. A stolen key of the bot can then put neither a sentence nor an
//! address into a card.

use messenger_core::traits::UiEvent;
use messenger_core::{DmInbound, Effect, Envelope};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use ts_rs::TS;

/// The name of the event of a card of the bot: `{"id", "t", "fields",
/// "from", "at"}`. `fields` are the values of the card as [`BotCard::read`]
/// took them, the same the row of the chat keeps: the module `cloud`, which
/// takes the event through the seam `BotCards`, gets nothing the chat
/// would not show.
pub const UI_EVENT_BOT_CARD: &str = "cloud.card";

/// The longest word of a card (a request id is 32).
const MAX_WORD: usize = 64;
/// The largest number a card carries: what JavaScript keeps exactly.
const MAX_NUMBER: u64 = (1 << 53) - 1;

/// A card as the chat shows it: the type and the values the table of the
/// contract names for it. A value that is absent or not what it should be
/// is `null`, and the card is drawn without it. Times are unix seconds,
/// sizes are bytes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "t")]
pub enum BotCard {
    #[serde(rename = "recovery.requested")]
    RecoveryRequested {
        /// What «Это не я — отменить» cancels.
        request_id: Option<String>,
        #[ts(type = "number | null")]
        created_at: Option<u64>,
        #[ts(type = "number | null")]
        expires_at: Option<u64>,
    },
    #[serde(rename = "recovery.escrow_read")]
    RecoveryEscrowRead {
        #[ts(type = "number | null")]
        at: Option<u64>,
    },
    #[serde(rename = "recovery.restored")]
    RecoveryRestored {
        #[ts(type = "number | null")]
        at: Option<u64>,
    },
    #[serde(rename = "account.deleted")]
    AccountDeleted {
        receipt: Option<String>,
        #[ts(type = "number | null")]
        at: Option<u64>,
    },
    #[serde(rename = "plan.expiring")]
    PlanExpiring {
        /// The id of the plan (`plus`), not a name to show as it is.
        plan: Option<String>,
        #[ts(type = "number | null")]
        until: Option<u64>,
    },
    #[serde(rename = "quota.90")]
    Quota90 {
        pool: Option<String>,
        #[ts(type = "number | null")]
        used: Option<u64>,
        #[ts(type = "number | null")]
        limit: Option<u64>,
    },
    #[serde(rename = "account.dormant")]
    AccountDormant {
        #[ts(type = "number | null")]
        since: Option<u64>,
        #[ts(type = "number | null")]
        delete_after: Option<u64>,
    },
    /// A type this app does not know: «Сообщение Veydan Cloud», no text.
    /// The type itself is the bot's word and is not handed on.
    #[serde(rename = "unknown")]
    Unknown,
}

/// A word of a card: an id, the id of a plan. Nothing a sentence or an
/// address could be written in.
fn is_word(s: &str) -> bool {
    !s.is_empty() && s.len() <= MAX_WORD && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// The receipt of a deletion: 12 characters of Crockford base32 in groups
/// of 4, `7K2M-ABCD-EFGH` (cloud-api.md §1). The one value of a card the
/// app shows as it came, so it is held to its shape: a word is not enough,
/// underscores make a sentence of it.
fn is_receipt(s: &str) -> bool {
    let crockford = |b: u8| matches!(b, b'0'..=b'9' | b'A'..=b'H' | b'J' | b'K' | b'M' | b'N' | b'P'..=b'T' | b'V'..=b'Z');
    s.len() == 14 && s.split('-').count() == 3 && s.split('-').all(|group| group.len() == 4 && group.bytes().all(crockford))
}

impl BotCard {
    /// The card of an envelope of the bot. Every type is a card; one that
    /// is not in the table is [`BotCard::Unknown`].
    pub fn read(envelope: &Envelope) -> Self {
        let word = |key: &str| envelope.str_field(key).filter(|s| is_word(s)).map(String::from);
        let receipt = |key: &str| envelope.str_field(key).filter(|s| is_receipt(s)).map(String::from);
        let number = |key: &str| envelope.fields.get(key).and_then(Value::as_u64).filter(|n| *n <= MAX_NUMBER);
        match envelope.t.as_str() {
            "recovery.requested" => Self::RecoveryRequested {
                request_id: word("request_id"),
                created_at: number("created_at"),
                expires_at: number("expires_at"),
            },
            "recovery.escrow_read" => Self::RecoveryEscrowRead { at: number("at") },
            "recovery.restored" => Self::RecoveryRestored { at: number("at") },
            "account.deleted" => Self::AccountDeleted { receipt: receipt("receipt"), at: number("at") },
            "plan.expiring" => Self::PlanExpiring { plan: word("plan"), until: number("until") },
            "quota.90" => Self::Quota90 { pool: word("pool"), used: number("used"), limit: number("limit") },
            "account.dormant" => Self::AccountDormant { since: number("since"), delete_after: number("delete_after") },
            _ => Self::Unknown,
        }
    }

    /// A card as a row keeps it (`media_json`). What a newer app wrote and
    /// this one cannot read is an unknown card, never an error.
    pub fn stored(json: &str) -> Self {
        serde_json::from_str(json).unwrap_or(Self::Unknown)
    }

    /// The values of the card by their names, without its type and without
    /// what it does not have: the `fields` of [`UI_EVENT_BOT_CARD`].
    fn fields(&self) -> Map<String, Value> {
        let mut fields = match serde_json::to_value(self) {
            Ok(Value::Object(map)) => map,
            _ => Map::new(),
        };
        fields.remove("t");
        fields.retain(|_, value| !value.is_null());
        fields
    }
}

/// The event [`UI_EVENT_BOT_CARD`] of the card `msg` carries in `envelope`.
/// The type is the bot's word (the module knows types this crate may not,
/// and shows none); the fields are the card's, never the envelope's: a
/// sentence written into one stops here, as it does for the chat.
pub(crate) fn card_event(msg: &DmInbound, envelope: &Envelope) -> Effect {
    Effect::Emit(UiEvent {
        name: UI_EVENT_BOT_CARD.into(),
        payload: serde_json::json!({
            "id": msg.rumor_id.as_hex(),
            "t": envelope.t,
            "fields": BotCard::read(envelope).fields(),
            "from": msg.sender.as_hex(),
            "at": msg.created_at.secs(),
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(json: &str) -> BotCard {
        BotCard::read(&Envelope::parse(json).unwrap())
    }

    #[test]
    fn a_card_keeps_the_values_of_its_type_and_nothing_else() {
        let id = "9c".repeat(16);
        let card = read(&format!(
            r#"{{"v":1,"t":"recovery.requested","request_id":"{id}","created_at":1791012345,"expires_at":1791014145,"text":"go to evil.example","url":"https://evil.example"}}"#
        ));
        assert_eq!(
            card,
            BotCard::RecoveryRequested { request_id: Some(id.clone()), created_at: Some(1_791_012_345), expires_at: Some(1_791_014_145) }
        );
        // The UI gets the type and those values, as a row keeps them.
        let json = serde_json::to_value(&card).unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "t": "recovery.requested", "request_id": id, "created_at": 1_791_012_345u64, "expires_at": 1_791_014_145u64 })
        );
        assert_eq!(BotCard::stored(&json.to_string()), card);

        assert_eq!(read(r#"{"v":1,"t":"recovery.escrow_read","at":5}"#), BotCard::RecoveryEscrowRead { at: Some(5) });
        assert_eq!(read(r#"{"v":1,"t":"recovery.restored"}"#), BotCard::RecoveryRestored { at: None });
        assert_eq!(
            read(r#"{"v":1,"t":"account.deleted","receipt":"AB7K-2QZD-9MXP","at":7}"#),
            BotCard::AccountDeleted { receipt: Some("AB7K-2QZD-9MXP".into()), at: Some(7) }
        );
        assert_eq!(
            read(r#"{"v":1,"t":"plan.expiring","plan":"plus","until":9}"#),
            BotCard::PlanExpiring { plan: Some("plus".into()), until: Some(9) }
        );
        assert_eq!(
            read(r#"{"v":1,"t":"quota.90","pool":"files","used":90,"limit":100}"#),
            BotCard::Quota90 { pool: Some("files".into()), used: Some(90), limit: Some(100) }
        );
        assert_eq!(
            read(r#"{"v":1,"t":"account.dormant","since":1,"delete_after":2}"#),
            BotCard::AccountDormant { since: Some(1), delete_after: Some(2) }
        );
    }

    #[test]
    fn a_value_that_is_not_a_word_or_a_number_is_left_out() {
        // A sentence, an address, a number as text, a negative or huge one.
        let card = read(r#"{"v":1,"t":"plan.expiring","plan":"Pay at evil.example","until":"soon"}"#);
        assert_eq!(card, BotCard::PlanExpiring { plan: None, until: None });
        let card = read(r#"{"v":1,"t":"account.deleted","receipt":"https://evil.example/x","at":-1}"#);
        assert_eq!(card, BotCard::AccountDeleted { receipt: None, at: None });
        let card = read(&format!(r#"{{"v":1,"t":"quota.90","pool":"{}","used":1e300,"limit":18446744073709551615}}"#, "a".repeat(65)));
        assert_eq!(card, BotCard::Quota90 { pool: None, used: None, limit: None });
        assert!(is_word("AB7K-2QZD_9"));
        for bad in ["", "a b", "a.b", "a/b", "a:b", "слово", "a\u{202e}b"] {
            assert!(!is_word(bad), "{bad:?}");
        }
    }

    #[test]
    fn a_receipt_is_a_receipt_and_not_a_word() {
        for good in ["AB7K-2QZD-9MXP", "7K2M-ABCD-EFGH", "0000-0000-0000"] {
            assert!(is_receipt(good), "{good}");
        }
        // A sentence through underscores is a word; a receipt it is not.
        // Nor are the letters Crockford leaves out, small letters, another
        // grouping or length.
        for bad in [
            "",
            "Account_suspended_enter_your_12_words_at_evil",
            "AB7K-2QZD-9MXL",
            "AB7K-2QZD-9MXI",
            "AB7K-2QZD-9MXO",
            "AB7K-2QZD-9MXU",
            "ab7k-2qzd-9mxp",
            "AB7K2QZD9MXP",
            "AB7K-2QZD-9MXP-AB7K",
            "AB7K_2QZD_9MXP",
            "AB7-K2QZD-9MXP",
            "AB7K-2QZD-9MX\u{202e}",
        ] {
            assert!(!is_receipt(bad), "{bad:?}");
        }
        let card = read(r#"{"v":1,"t":"account.deleted","receipt":"Account_suspended_enter_your_12_words_at_evil","at":7}"#);
        assert_eq!(card, BotCard::AccountDeleted { receipt: None, at: Some(7) });
    }

    /// The `fields` of the event of the card `json` is.
    fn event_fields(json: &str) -> Value {
        let msg = DmInbound {
            envelope: messenger_core::inbound::Envelope {
                wire_id: messenger_core::EventId::parse(&"1".repeat(64)).unwrap(),
                source: messenger_core::EventSource::Relay { url: messenger_core::RelayUrl::parse("wss://relay.example.com").unwrap() },
                wire_created_at: messenger_core::Timestamp(10),
            },
            rumor_id: messenger_core::EventId::parse(&"2".repeat(64)).unwrap(),
            sender: messenger_core::PubKey::parse(&"ab".repeat(32)).unwrap(),
            recipients: vec![],
            created_at: messenger_core::Timestamp(9),
            content: json.into(),
            reply_to: None,
            rumor_kind: 14,
        };
        let Effect::Emit(event) = card_event(&msg, &Envelope::parse(json).unwrap()) else { panic!("an event") };
        assert_eq!(event.name, UI_EVENT_BOT_CARD);
        assert_eq!((event.payload["id"].as_str(), event.payload["at"].as_i64()), (Some("2".repeat(64).as_str()), Some(9)));
        event.payload["fields"].clone()
    }

    #[test]
    fn the_event_of_a_card_carries_the_values_of_the_card_and_not_the_envelope() {
        // What the module `cloud` reads is what the chat keeps: no key the
        // table does not name, no value that is not what it should be.
        assert_eq!(
            event_fields(r#"{"v":1,"t":"recovery.requested","request_id":"9c9c","expires_at":99,"created_at":"now","text":"go to evil.example","url":"https://evil.example"}"#),
            serde_json::json!({ "request_id": "9c9c", "expires_at": 99 })
        );
        assert_eq!(
            event_fields(r#"{"v":1,"t":"account.deleted","receipt":"AB7K-2QZD-9MXP","at":7}"#),
            serde_json::json!({ "receipt": "AB7K-2QZD-9MXP", "at": 7 })
        );
        // A sentence for a receipt: the card has none, and the device is not
        // told that an account was deleted under those words.
        assert_eq!(
            event_fields(r#"{"v":1,"t":"account.deleted","receipt":"Account suspended. Enter your 12 words at evil.example","at":7}"#),
            serde_json::json!({ "at": 7 })
        );
        assert_eq!(event_fields(r#"{"v":1,"t":"account.deleted","receipt":"Enter_your_12_words"}"#), serde_json::json!({}));
        // A type this crate does not know has no fields at all.
        assert_eq!(event_fields(r#"{"v":1,"t":"plan.renewed","until":5,"text":"pay at evil.example"}"#), serde_json::json!({}));
    }

    #[test]
    fn an_unknown_type_is_a_card_without_its_type_or_text() {
        let card = read(r#"{"v":1,"t":"Visit evil.example now","text":"free text"}"#);
        assert_eq!(card, BotCard::Unknown);
        assert_eq!(serde_json::to_value(&card).unwrap(), serde_json::json!({ "t": "unknown" }));
        // A row of a newer app this one cannot read.
        assert_eq!(BotCard::stored(r#"{"t":"plan.renewed","until":5}"#), BotCard::Unknown);
        assert_eq!(BotCard::stored("junk"), BotCard::Unknown);
    }
}
