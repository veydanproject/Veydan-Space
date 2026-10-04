//! The XML of a WinRT toast. Plain text work, so it is tested everywhere.

use std::path::Path;

use crate::Toast;

/// The toast: title, the lines, the sender's face in a circle, and the
/// sound of a message (or none).
pub(crate) fn toast_xml(t: &Toast) -> String {
    let image = t
        .image
        .as_deref()
        .map(|p| format!(r#"<image placement="appLogoOverride" hint-crop="circle" src="{}"/>"#, attr(&file_uri(p))))
        .unwrap_or_default();
    let audio = if t.silent {
        r#"<audio silent="true"/>"#.to_string()
    } else {
        r#"<audio src="ms-winsoundevent:Notification.IM"/>"#.to_string()
    };
    format!(
        r#"<toast><visual><binding template="ToastGeneric"><text>{}</text><text>{}</text>{image}</binding></visual>{audio}</toast>"#,
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

    #[test]
    fn toast_xml_escapes_and_goes_silent() {
        let t = Toast {
            key: "dm:a".into(),
            title: "A & B".into(),
            body: "<hi>".into(),
            image: Some(r"C:\cache\a b.png".into()),
            silent: true,
        };
        let xml = toast_xml(&t);
        assert!(xml.contains("<text>A &amp; B</text><text>&lt;hi&gt;</text>"));
        assert!(xml.contains(r#"src="file:///C:/cache/a b.png""#));
        assert!(xml.contains(r#"<audio silent="true"/>"#));
    }
}
