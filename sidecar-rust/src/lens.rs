use std::path::Path;

const MAX_HTML_BYTES: usize = 512 * 1024;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Manifest {
    pub name: String,
    pub icon: String,
    pub view: String,
    pub source: String,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct LensSnapshot {
    pub id: String,
    pub name: String,
    pub icon: String,
    pub html: String,
}

pub fn validate(id: &str, name: &str, icon: &str, html: &str) -> Result<(), String> {
    if id.is_empty()
        || id.len() > 64
        || !id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        || id.starts_with('-')
        || id.ends_with('-')
    {
        return Err("Lens id must use 1-64 lowercase letters, numbers, or hyphens".to_string());
    }
    if name.trim().is_empty() || name.len() > 80 {
        return Err("Lens name must use 1-80 characters".to_string());
    }
    if icon.trim().is_empty() || icon.len() > 32 {
        return Err("Lens icon must use 1-32 characters".to_string());
    }
    if html.len() > MAX_HTML_BYTES {
        return Err("Lens HTML exceeds the 512 KiB limit".to_string());
    }
    let lowercase = html.to_ascii_lowercase();
    if !lowercase.contains("<html") || !lowercase.contains("</html>") {
        return Err("Lens HTML must be a complete HTML document".to_string());
    }
    Ok(())
}

pub fn write(
    mind_root: &Path,
    id: &str,
    name: &str,
    icon: &str,
    html: &str,
) -> Result<(), std::io::Error> {
    let dir = mind_root.join(".github").join("lens").join(id);
    std::fs::create_dir_all(&dir)?;

    let manifest = Manifest {
        name: name.to_string(),
        icon: icon.to_string(),
        view: "canvas".to_string(),
        source: "index.html".to_string(),
    };
    let manifest_bytes = serde_json::to_vec_pretty(&manifest)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

    std::fs::write(dir.join("index.html"), html)?;

    let mut manifest_data = manifest_bytes;
    manifest_data.push(b'\n');
    std::fs::write(dir.join("view.json"), manifest_data)?;

    Ok(())
}
