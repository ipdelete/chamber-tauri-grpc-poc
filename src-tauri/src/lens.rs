use serde::Serialize;

const MAX_HTML_BYTES: usize = 512 * 1024;

/// A lens snapshot the sidecar has already written to the mind directory.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct LensDefinition {
    pub id: String,
    pub name: String,
    pub icon: String,
    pub html: String,
}

/// Guard the renderer. The sidecar owns the lens, so this only decides whether
/// Chamber is willing to display what arrived.
pub fn validate(lens: &LensDefinition) -> Result<(), String> {
    if lens.id.is_empty()
        || lens.id.len() > 64
        || !lens
            .id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        || lens.id.starts_with('-')
        || lens.id.ends_with('-')
    {
        return Err("Lens id must use 1-64 lowercase letters, numbers, or hyphens".to_owned());
    }
    if lens.name.trim().is_empty() || lens.name.len() > 80 {
        return Err("Lens name must use 1-80 characters".to_owned());
    }
    if lens.icon.trim().is_empty() || lens.icon.len() > 32 {
        return Err("Lens icon must use 1-32 characters".to_owned());
    }
    if lens.html.len() > MAX_HTML_BYTES {
        return Err("Lens HTML exceeds the 512 KiB limit".to_owned());
    }
    let lowercase = lens.html.to_ascii_lowercase();
    if !lowercase.contains("<html") || !lowercase.contains("</html>") {
        return Err("Lens HTML must be a complete HTML document".to_owned());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lens(id: &str, html: &str) -> LensDefinition {
        LensDefinition {
            id: id.to_owned(),
            name: "Release Board".to_owned(),
            icon: "layout".to_owned(),
            html: html.to_owned(),
        }
    }

    #[test]
    fn accepts_a_complete_document() {
        let valid = lens(
            "release-board",
            "<!doctype html><html><body>Ready</body></html>",
        );
        assert_eq!(validate(&valid), Ok(()));
    }

    #[test]
    fn rejects_a_bad_id() {
        let invalid = lens("Release Board", "<html></html>");
        assert!(validate(&invalid).is_err());
    }

    #[test]
    fn rejects_a_fragment() {
        let invalid = lens("release-board", "<div>Ready</div>");
        assert!(validate(&invalid).is_err());
    }
}
