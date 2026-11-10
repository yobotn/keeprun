use anyhow::{Context, Result};
use std::path::Path;

/// Read the API token. Fails with a clear message when the agent has not created it yet.
pub fn read(path: &Path) -> Result<String> {
    let t = std::fs::read_to_string(path).with_context(|| {
        format!(
            "Cannot read the agent token at {}. Start the agent first, or run as a user that can read this file.",
            path.display()
        )
    })?;
    Ok(t.trim().to_string())
}

/// Read the token, or create a new random one when the file does not exist.
pub fn load_or_create(path: &Path) -> Result<String> {
    if path.is_file() {
        return read(path);
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("Cannot create {}", dir.display()))?;
    }
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|e| anyhow::anyhow!("Cannot generate a token: {e}"))?;
    let token: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    std::fs::write(path, &token).with_context(|| format!("Cannot write {}", path.display()))?;
    Ok(token)
}
