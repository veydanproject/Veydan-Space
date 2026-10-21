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
    /// The paid plan ended and the account is on free: nothing is deleted,
    /// new files wait until there is room.
    #[serde(rename = "account.downgraded")]
    AccountDowngraded {
        /// The id of the plan that ended.
        plan: Option<String>,
        /// The limits of free, bytes.
        #[ts(type = "number | null")]
        files_bytes: Option<u64>,
        #[ts(type = "number | null")]
        text_bytes: Option<u64>,
    },
    /// A code to sign in to the web cabinet (cloud-api.md §16.5), sent only
    /// in answer to a request on the site; «Это не я» denies it.
    #[serde(rename = "web.login")]
    WebLogin {
        /// The sign-in, 16 hex: what «Это не я» denies.
        id: Option<String>,
        /// Exactly 6 digits, as text (leading zeros kept); without it the
        /// card shows no code.
        code: Option<String>,
        /// Two capital letters (ISO 3166-1 alpha-2) of where the request came from.
        country: Option<String>,
        /// The family of the browser: `chrome`, `firefox`, `safari`, `edge`, `opera`, `other`.
        ua_family: Option<String>,
        #[ts(type = "number | null")]
        expires_at: Option<u64>,
    },
    /// A browser signed in to the web cabinet; «Завершить сессию» ends it.
    #[serde(rename = "web.signed_in")]
    WebSignedIn {
        /// The session, 16 hex.
        session: Option<String>,
        country: Option<String>,
        ua_family: Option<String>,
        #[ts(type = "number | null")]
        at: Option<u64>,
    },
    /// The web cabinet asks the app to confirm a dangerous action.
    #[serde(rename = "web.confirm")]
    WebConfirm {
        /// The action waiting, 16 hex.
        id: Option<String>,
        /// `delete`: the deletion of the account.
        action: Option<String>,
        country: Option<String>,
        ua_family: Option<String>,
        #[ts(type = "number | null")]
        expires_at: Option<u64>,
    },
    /// A plan bought for oneself in the web cabinet was paid and applied.
    #[serde(rename = "order.paid")]
    OrderPaid {
        /// The order, 16 hex.
        order_id: Option<String>,
        /// The id of the plan.
        plan: Option<String>,
        #[ts(type = "number | null")]
        period_days: Option<u64>,
        #[ts(type = "number | null")]
        until: Option<u64>,
        /// 3 to 5 capital letters (`USD`, `USDT`).
        currency: Option<String>,
        #[ts(type = "number | null")]
        amount_minor: Option<u64>,
        #[ts(type = "number | null")]
        at: Option<u64>,
    },
    /// An activation key bought in the web cabinet: the card is the purchase.
    #[serde(rename = "order.key")]
    OrderKey {
        order_id: Option<String>,
        /// 5 groups of 4 Crockford characters joined by `-`.
        key: Option<String>,
        plan: Option<String>,
        #[ts(type = "number | null")]
        period_days: Option<u64>,
        currency: Option<String>,
        #[ts(type = "number | null")]
        amount_minor: Option<u64>,
        #[ts(type = "number | null")]
        at: Option<u64>,
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
    s.len() == 14 && s.split('-').count() == 3 && s.split('-').all(|group| group.len() == 4 && group.bytes().all(is_crockford))
}

/// A character of Crockford base32 in upper case (no I, L, O, U).
fn is_crockford(b: u8) -> bool {
    matches!(b, b'0'..=b'9' | b'A'..=b'H' | b'J' | b'K' | b'M' | b'N' | b'P'..=b'T' | b'V'..=b'Z')
}

/// The code of a sign-in to the web cabinet: exactly 6 ASCII digits, kept
/// as text so a leading zero stays (cloud-api.md §16.2). The card shows it
/// large, so nothing else may pass for it.
pub fn is_login_code(s: &str) -> bool {
    s.len() == 6 && s.bytes().all(|b| b.is_ascii_digit())
}

/// An activation key: 5 groups of 4 Crockford characters joined by `-`
/// (`7K2M-ABCD-EFGH-JKMN-PQRS`). The purchase card shows it to be copied.
pub fn is_activation_key(s: &str) -> bool {
    s.len() == 24 && s.split('-').count() == 5 && s.split('-').all(|group| group.len() == 4 && group.bytes().all(is_crockford))
}

/// A country: two capital ASCII letters (ISO 3166-1 alpha-2).
pub fn is_country(s: &str) -> bool {
    s.len() == 2 && s.bytes().all(|b| b.is_ascii_uppercase())
}

/// The families of browsers the cabinet names (cloud-api.md §16.1).
pub const UA_FAMILIES: [&str; 6] = ["chrome", "firefox", "safari", "edge", "opera", "other"];

/// A family of a browser, one word of [`UA_FAMILIES`].
pub fn is_ua_family(s: &str) -> bool {
    UA_FAMILIES.contains(&s)
}

/// The id of a sign-in, a session, an action or an order: 16 hex in lower case.
pub fn is_id16(s: &str) -> bool {
    s.len() == 16 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// A currency of a price: 3 to 5 capital letters (`USD`, `USDT`), as vcloud keeps them.
pub fn is_currency(s: &str) -> bool {
    (3..=5).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_uppercase())
}

/// The actions the web cabinet asks the app to confirm.
const WEB_ACTIONS: [&str; 1] = ["delete"];

impl BotCard {
    /// The card of an envelope of the bot. Every type is a card; one that
    /// is not in the table is [`BotCard::Unknown`].
    pub fn read(envelope: &Envelope) -> Self {
        let word = |key: &str| envelope.str_field(key).filter(|s| is_word(s)).map(String::from);
        let receipt = |key: &str| envelope.str_field(key).filter(|s| is_receipt(s)).map(String::from);
        let number = |key: &str| envelope.fields.get(key).and_then(Value::as_u64).filter(|n| *n <= MAX_NUMBER);
        // A value held to an exact shape: the card shows it, or acts on it.
        let shaped = |key: &str, ok: fn(&str) -> bool| envelope.str_field(key).filter(|s| ok(s)).map(String::from);
        let id = |key: &str| shaped(key, is_id16);
        let country = || shaped("country", is_country);
        let ua_family = || shaped("ua_family", is_ua_family);
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
            "account.downgraded" => Self::AccountDowngraded { plan: word("plan"), files_bytes: number("files_bytes"), text_bytes: number("text_bytes") },
            "web.login" => Self::WebLogin {
                id: id("id"),
                code: shaped("code", is_login_code),
                country: country(),
                ua_family: ua_family(),
                expires_at: number("expires_at"),
            },
            "web.signed_in" => Self::WebSignedIn { session: id("session"), country: country(), ua_family: ua_family(), at: number("at") },
            "web.confirm" => Self::WebConfirm {
                id: id("id"),
                action: shaped("action", |a| WEB_ACTIONS.contains(&a)),
                country: country(),
                ua_family: ua_family(),
                expires_at: number("expires_at"),
            },
            "order.paid" => Self::OrderPaid {
                order_id: id("order_id"),
                plan: word("plan"),
                period_days: number("period_days"),
                until: number("until"),
                currency: shaped("currency", is_currency),
                amount_minor: number("amount_minor"),
                at: number("at"),
            },
            "order.key" => Self::OrderKey {
                order_id: id("order_id"),
                key: shaped("key", is_activation_key),
                plan: word("plan"),
                period_days: number("period_days"),
                currency: shaped("currency", is_currency),
                amount_minor: number("amount_minor"),
                at: number("at"),
            },
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
        assert_eq!(
            read(r#"{"v":1,"t":"account.downgraded","plan":"pro","files_bytes":2147483648,"text_bytes":536870912}"#),
            BotCard::AccountDowngraded { plan: Some("pro".into()), files_bytes: Some(2_147_483_648), text_bytes: Some(536_870_912) }
        );
        assert_eq!(
            read(r#"{"v":1,"t":"account.downgraded","plan":"Delete at evil.example","files_bytes":-1}"#),
            BotCard::AccountDowngraded { plan: None, files_bytes: None, text_bytes: None }
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
            srv: None,
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
    fn a_login_code_is_six_digits_as_text() {
        for good in ["123456", "000000", "012345"] {
            assert!(is_login_code(good), "{good}");
        }
        for bad in ["", "12345", "1234567", "12 456", "12345a", "١٢٣٤٥٦", "123456\u{200b}", "-12345", "+12345"] {
            assert!(!is_login_code(bad), "{bad:?}");
        }
        // A number for a code is not a code: the leading zero would be lost.
        let card = read(r#"{"v":1,"t":"web.login","id":"0123456789abcdef","code":12345,"expires_at":5}"#);
        assert_eq!(card, BotCard::WebLogin { id: Some("0123456789abcdef".into()), code: None, country: None, ua_family: None, expires_at: Some(5) });
    }

    #[test]
    fn an_activation_key_is_five_groups_of_crockford() {
        for good in ["7K2M-ABCD-EFGH-JKMN-PQRS", "0000-0000-0000-0000-0000", "VWXY-Z123-4567-89AB-CDEF"] {
            assert!(is_activation_key(good), "{good}");
        }
        for bad in [
            "",
            "7K2M-ABCD-EFGH-JKMN",
            "7K2M-ABCD-EFGH-JKMN-PQRS-TVWX",
            "7k2m-abcd-efgh-jkmn-pqrs",
            "7K2M-ABCD-EFGH-JKMN-PQRI",
            "7K2M-ABCD-EFGH-JKMN-PQRL",
            "7K2M-ABCD-EFGH-JKMN-PQRO",
            "7K2M-ABCD-EFGH-JKMN-PQRU",
            "7K2MABCDEFGHJKMNPQRS",
            "7K2M_ABCD_EFGH_JKMN_PQRS",
            "7K2-MABCD-EFGH-JKMN-PQRS",
            "Visit-evil-exam-ple.-com!",
        ] {
            assert!(!is_activation_key(bad), "{bad:?}");
        }
    }

    #[test]
    fn a_country_a_browser_and_an_id_are_held_to_their_shape() {
        assert!(is_country("DE") && is_country("RU"));
        for bad in ["", "D", "de", "DEU", "D1", "ДЕ"] {
            assert!(!is_country(bad), "{bad:?}");
        }
        for good in UA_FAMILIES {
            assert!(is_ua_family(good), "{good}");
        }
        for bad in ["", "Chrome", "chromium", "Mozilla/5.0", "other "] {
            assert!(!is_ua_family(bad), "{bad:?}");
        }
        assert!(is_id16("0123456789abcdef"));
        for bad in ["", "0123456789ABCDEF", "0123456789abcde", "0123456789abcdef0", "0123456789abcdeg"] {
            assert!(!is_id16(bad), "{bad:?}");
        }
        assert!(is_currency("USD") && is_currency("USDT"));
        for bad in ["", "US", "usd", "USDTXX", "U$D"] {
            assert!(!is_currency(bad), "{bad:?}");
        }
    }

    #[test]
    fn the_cards_of_the_web_cabinet_keep_their_values() {
        let id = "0123456789abcdef";
        assert_eq!(
            read(&format!(r#"{{"v":1,"t":"web.login","id":"{id}","code":"012345","country":"DE","ua_family":"chrome","expires_at":9,"text":"enter at evil.example"}}"#)),
            BotCard::WebLogin { id: Some(id.into()), code: Some("012345".into()), country: Some("DE".into()), ua_family: Some("chrome".into()), expires_at: Some(9) }
        );
        // A sentence in any of them: nothing of it reaches the card.
        assert_eq!(
            read(r#"{"v":1,"t":"web.login","id":"enter_at_evil","code":"Enter at evil.example","country":"Germany","ua_family":"Mozilla/5.0"}"#),
            BotCard::WebLogin { id: None, code: None, country: None, ua_family: None, expires_at: None }
        );
        assert_eq!(
            read(&format!(r#"{{"v":1,"t":"web.signed_in","session":"{id}","country":"FR","ua_family":"firefox","at":3}}"#)),
            BotCard::WebSignedIn { session: Some(id.into()), country: Some("FR".into()), ua_family: Some("firefox".into()), at: Some(3) }
        );
        assert_eq!(
            read(&format!(r#"{{"v":1,"t":"web.confirm","id":"{id}","action":"delete","ua_family":"safari","expires_at":4}}"#)),
            BotCard::WebConfirm { id: Some(id.into()), action: Some("delete".into()), country: None, ua_family: Some("safari".into()), expires_at: Some(4) }
        );
        // An action this app does not know is no action to confirm.
        assert_eq!(
            read(&format!(r#"{{"v":1,"t":"web.confirm","id":"{id}","action":"transfer"}}"#)),
            BotCard::WebConfirm { id: Some(id.into()), action: None, country: None, ua_family: None, expires_at: None }
        );
        assert_eq!(
            read(&format!(r#"{{"v":1,"t":"order.paid","order_id":"{id}","plan":"plus","period_days":30,"until":99,"currency":"USDT","amount_minor":400,"at":5}}"#)),
            BotCard::OrderPaid {
                order_id: Some(id.into()), plan: Some("plus".into()), period_days: Some(30), until: Some(99),
                currency: Some("USDT".into()), amount_minor: Some(400), at: Some(5),
            }
        );
        let card = read(&format!(r#"{{"v":1,"t":"order.key","order_id":"{id}","key":"7K2M-ABCD-EFGH-JKMN-PQRS","plan":"pro","period_days":365,"currency":"EUR","amount_minor":4990,"at":6}}"#));
        assert_eq!(
            card,
            BotCard::OrderKey {
                order_id: Some(id.into()), key: Some("7K2M-ABCD-EFGH-JKMN-PQRS".into()), plan: Some("pro".into()), period_days: Some(365),
                currency: Some("EUR".into()), amount_minor: Some(4990), at: Some(6),
            }
        );
        assert_eq!(BotCard::stored(&serde_json::to_string(&card).unwrap()), card);
        assert_eq!(
            read(r#"{"v":1,"t":"order.key","key":"Pay_at_evil_example","currency":"dollars","amount_minor":-1}"#),
            BotCard::OrderKey { order_id: None, key: None, plan: None, period_days: None, currency: None, amount_minor: None, at: None }
        );
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
