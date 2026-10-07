//! Filesystem-aware containment for transcript working-directory scopes.
use std::path::Path;

/// Accept only existing absolute scopes contained in the canonical project.
pub fn matches_project(scope: Option<&str>, project: &Path) -> bool {
    let Some(scope) = scope.map(Path::new).filter(|path| path.is_absolute()) else {
        return false;
    };
    match (scope.canonicalize(), project.canonicalize()) {
        (Ok(scope), Ok(project)) => scope.starts_with(project),
        _ => false,
    }
}

/// Read only the leading provenance records, without parsing a whole transcript.
/// Unknown provenance fails closed rather than guessing from prompt text.
pub fn transcript_scope(path: &Path, kind: &str) -> std::io::Result<Option<String>> {
    use std::io::{BufRead, Read};
    let reader = std::io::BufReader::new(std::fs::File::open(path)?.take(64 * 1024));
    for line in reader.lines().take(64) {
        let line = line?;
        if !line.contains("\"cwd\"") {
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        let scope = if kind == "codex" {
            if value.get("type").and_then(|v| v.as_str()) != Some("session_meta") {
                continue;
            }
            value.get("payload").and_then(|v| v.get("cwd"))
        } else {
            value.get("cwd")
        };
        if let Some(scope) = scope.and_then(|v| v.as_str()) {
            return Ok(Some(scope.to_owned()));
        }
    }
    Ok(None)
}

#[cfg(test)]
#[path = "scope_tests.rs"]
mod tests;
