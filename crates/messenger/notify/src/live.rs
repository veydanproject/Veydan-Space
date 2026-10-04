// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! A notice of the running app, told the way a push is told.
//!
//! While the app runs, it takes messages in itself and says so with a
//! `messenger_core::Notice`: own, old, deleted messages and muted chats
//! are already left out. A computer shows that without any push; this
//! turns it into the same [`Outcome`] the phone's handler gives, so both
//! word a notification from one set of facts and the same settings.

use crate::describe::https;
use crate::notice::{Body, ChatKind, Notice, Outcome, Plain};
use crate::settings::{Content, Settings};

/// Who wrote, as the app would show them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Face {
    pub name: String,
    pub picture: Option<String>,
}

/// `face` is the sender's. `locked`: a PIN guards the app, so, as on a
/// phone that gets no keys, only that something came is said.
pub fn live(notice: &messenger_core::Notice, face: Face, settings: &Settings, locked: bool) -> Outcome {
    let group = notice.chat_id.as_deref().is_some_and(|c| c.starts_with("group:"));
    // An invite names a group, and comes from a person outside it.
    let invite = matches!(notice.body, Some(Body::Invite { .. }));
    let kind = if group && !invite {
        ChatKind::Group
    } else if notice.request {
        ChatKind::Request
    } else {
        ChatKind::Dm
    };

    if locked || settings.content == Content::None {
        return Outcome::Plain(Plain {
            kind,
            chat: notice.chat_id.clone(),
            title: (kind == ChatKind::Group).then(|| notice.title.clone()),
            muted: false,
            count: 1,
        });
    }

    // An invite comes from a person, not from the group it names.
    let title = if kind == ChatKind::Group { notice.title.clone() } else { face.name.clone() };
    Outcome::Show(Notice {
        kind,
        chat: notice.chat_id.clone(),
        title,
        sender: face.name,
        sender_key: notice.sender.clone().unwrap_or_default(),
        picture: https(face.picture),
        body: notice.body.clone().filter(|_| settings.content == Content::SenderText),
        muted: false,
        hide_on_lockscreen: settings.lockscreen_hidden,
        count: 1,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notice(chat: Option<&str>, body: Option<Body>) -> messenger_core::Notice {
        messenger_core::Notice {
            title: "Team".into(),
            body,
            chat_id: chat.map(str::to_string),
            sender: Some("ab".repeat(32)),
            request: false,
        }
    }

    fn text(t: &str) -> Option<Body> {
        Some(Body::Text { text: t.into() })
    }

    fn face() -> Face {
        Face { name: "Alice".into(), picture: Some("https://x/a.png".into()) }
    }

    fn settings(content: Content) -> Settings {
        Settings { content, lockscreen_hidden: false }
    }

    fn shown(o: Outcome) -> Notice {
        match o {
            Outcome::Show(n) => n,
            other => panic!("expected a notice, got {other:?}"),
        }
    }

    #[test]
    fn group_message_names_the_group_and_the_author() {
        let n = shown(live(&notice(Some("group:g"), text("hi")), face(), &settings(Content::SenderText), false));
        assert_eq!(n.kind, ChatKind::Group);
        assert_eq!(n.title, "Team");
        assert_eq!(n.sender, "Alice");
        assert_eq!(n.body, Some(Body::Text { text: "hi".into() }));
        assert_eq!(n.picture.as_deref(), Some("https://x/a.png"));
    }

    #[test]
    fn sender_only_keeps_the_text_in_the_app() {
        let n = shown(live(&notice(Some("dm:p"), text("secret")), face(), &settings(Content::Sender), false));
        assert_eq!(n.kind, ChatKind::Dm);
        assert_eq!(n.title, "Alice");
        assert_eq!(n.body, None);
    }

    #[test]
    fn nothing_and_a_lock_say_only_that_something_came() {
        for (content, locked) in [(Content::None, false), (Content::SenderText, true)] {
            match live(&notice(Some("group:g"), text("secret")), face(), &settings(content), locked) {
                Outcome::Plain(p) => {
                    assert_eq!(p.title.as_deref(), Some("Team"));
                    assert_eq!(p.chat.as_deref(), Some("group:g"));
                }
                other => panic!("expected plain, got {other:?}"),
            }
        }
        match live(&notice(Some("dm:p"), text("secret")), face(), &settings(Content::None), false) {
            Outcome::Plain(p) => assert_eq!(p.title, None, "a person's name is not given away"),
            other => panic!("expected plain, got {other:?}"),
        }
    }

    #[test]
    fn an_invite_comes_from_a_person_and_a_welcome_from_the_group() {
        let n = shown(live(&notice(None, Some(Body::Invite { group_name: "Team".into() })), face(), &settings(Content::SenderText), false));
        assert_eq!(n.kind, ChatKind::Dm);
        assert_eq!(n.title, "Alice", "an invite comes from a person");
        assert_eq!(n.body, Some(Body::Invite { group_name: "Team".into() }));
        let n = shown(live(&notice(Some("group:g"), Some(Body::Welcome { group_name: "Team".into() })), face(), &settings(Content::SenderText), false));
        assert_eq!(n.kind, ChatKind::Group);
        assert_eq!(n.body, Some(Body::Welcome { group_name: "Team".into() }));
    }

    #[test]
    fn a_request_is_marked_and_pictures_are_https_only() {
        let mut n0 = notice(Some("dm:p"), text("hello"));
        n0.request = true;
        let f = Face { name: "Bob".into(), picture: Some("http://x/b.png".into()) };
        let n = shown(live(&n0, f, &settings(Content::SenderText), false));
        assert_eq!(n.kind, ChatKind::Request);
        assert_eq!(n.picture, None);
    }
}
