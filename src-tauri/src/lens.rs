use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

const MAX_HTML_BYTES: usize = 512 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LensDefinition {
    pub id: String,
    pub name: String,
    pub icon: String,
    pub html: String,
}

#[derive(Serialize)]
struct LensManifest<'a> {
    name: &'a str,
    icon: &'a str,
    view: &'static str,
    source: &'static str,
}

pub fn upsert(root: &Path, arguments_json: &str) -> Result<LensDefinition, String> {
    let lens: LensDefinition =
        serde_json::from_str(arguments_json).map_err(|error| error.to_string())?;
    validate(&lens)?;

    let directory = root.join(&lens.id);
    fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    fs::write(directory.join("index.html"), &lens.html).map_err(|error| error.to_string())?;
    let manifest = serde_json::to_string_pretty(&LensManifest {
        name: &lens.name,
        icon: &lens.icon,
        view: "canvas",
        source: "index.html",
    })
    .map_err(|error| error.to_string())?;
    fs::write(directory.join("view.json"), format!("{manifest}\n"))
        .map_err(|error| error.to_string())?;

    Ok(lens)
}

fn validate(lens: &LensDefinition) -> Result<(), String> {
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

    #[test]
    fn persists_a_valid_canvas_lens() {
        let root = std::env::temp_dir().join(format!(
            "chamber-lens-test-{}",
            crate::agent_runtime::generate_auth_token().unwrap()
        ));
        let lens = upsert(
            &root,
            r#"{"id":"release-board","name":"Release Board","icon":"layout","html":"<!doctype html><html><body>Ready</body></html>"}"#,
        )
        .unwrap();

        assert_eq!(lens.id, "release-board");
        assert!(root.join("release-board/index.html").is_file());
        assert!(
            fs::read_to_string(root.join("release-board/view.json"))
                .unwrap()
                .contains(r#""view": "canvas""#)
        );

        fs::remove_dir_all(root).unwrap();
    }
}
