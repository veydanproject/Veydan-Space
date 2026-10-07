//! The XML of a WinRT toast. Plain text work, so it is tested everywhere.

use std::path::Path;

use crate::{Kind, Toast};

/// The toast: title, the lines, the sender's face in a circle, the
/// buttons, and the sound of a message (or none). A call rings in a loop
/// and stays on the screen (`incomingCall`) until it is taken away.
///
/// A button hands its id back as the activation's arguments; a click on
/// the toast itself hands back the toast's `launch`, which is left out:
/// empty arguments are a click.
pub(crate) fn toast_xml(t: &Toast) -> String {
    let image = t
        .image
        .as_deref()
        .map(|p| format!(r#"<image placement="appLogoOverride" hint-crop="circle" src="{}"/>"#, attr(&file_uri(p))))
        .unwrap_or_default();
    let audio = match (t.silent, t.kind) {
        (true, _) => r#"<audio silent="true"/>"#,
        (false, Kind::Message) => r#"<audio src="ms-winsoundevent:Notification.IM"/>"#,
        (false, Kind::Call) => r#"<audio src="ms-winsoundevent:Notification.Looping.Call" loop="true"/>"#,
    };
    let scenario = match t.kind {
        Kind::Message => "",
        Kind::Call => r#" scenario="incomingCall""#,
    };
    let actions = if t.actions.is_empty() {
        String::new()
    } else {
        let buttons: String = t
            .actions
            .iter()
            .map(|a| {
                format!(
                    r#"<action content="{}" arguments="{}" activationType="foreground"/>"#,
                    attr(&a.label),
                    attr(&a.id)
                )
            })
            .collect();
        format!("<actions>{buttons}</actions>")
    };
    format!(
        r#"<toast{scenario}><visual><binding template="ToastGeneric"><text>{}</text><text>{}</text>{image}</binding></visual>{actions}{audio}</toast>"#,
        text(&t.title),
        text(&t.body),
    )
}

fn file_uri(path: &Path) -> String {
    format!("file:///{}", path.to_string_lossy().replace('\\', "/"))
}

fn text(s: &str) -> String {
    crate::escape(s)
}

fn attr(s: &str) -> String {
    crate::escape(s).replace('"', "&quot;").replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Action;

    #[test]
    fn toast_xml_escapes_and_goes_silent() {
        let t = Toast {
            key: "dm:a".into(),
            title: "A & B".into(),
            body: "<hi>".into(),
            image: Some(r"C:\cache\a b.png".into()),
            silent: true,
            ..Toast::default()
        };
        let xml = toast_xml(&t);
        assert!(xml.starts_with("<toast><visual>"), "{xml}");
        assert!(xml.contains("<text>A &amp; B</text><text>&lt;hi&gt;</text>"));
        assert!(xml.contains(r#"src="file:///C:/cache/a b.png""#));
        assert!(xml.contains(r#"<audio silent="true"/>"#));
        assert!(!xml.contains("<actions>"));
    }

    #[test]
    fn a_call_rings_in_a_loop_with_its_buttons() {
        let t = Toast {
            key: "call:c1".into(),
            title: "Alice".into(),
            body: "Incoming video call".into(),
            kind: Kind::Call,
            actions: vec![Action::new("answer", "Ответить"), Action::new("decline", "\"No\"")],
            ..Toast::default()
        };
        let xml = toast_xml(&t);
        assert_eq!(
            xml,
            concat!(
                r#"<toast scenario="incomingCall"><visual><binding template="ToastGeneric">"#,
                r#"<text>Alice</text><text>Incoming video call</text></binding></visual>"#,
                r#"<actions><action content="Ответить" arguments="answer" activationType="foreground"/>"#,
                r#"<action content="&quot;No&quot;" arguments="decline" activationType="foreground"/></actions>"#,
                r#"<audio src="ms-winsoundevent:Notification.Looping.Call" loop="true"/></toast>"#,
            )
        );
        let quiet = toast_xml(&Toast { silent: true, ..t });
        assert!(quiet.contains(r#"<audio silent="true"/>"#) && quiet.contains("<actions>"));
    }
}
