//! Relative paths that are valid on every supported platform (macOS, Windows, Linux).
//!
//! Every path written into an index document is `/`-separated, relative, and passes
//! [`check_relative`]. Bundle names become cache and library directories, so they are checked
//! rather than silently rewritten: a name that cannot be stored portably is an error to surface.

const WINDOWS_RESERVED: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Why a path component is not portable, or `None` when it is fine everywhere.
pub fn component_problem(component: &str) -> Option<&'static str> {
    if component.is_empty() || component == "." || component == ".." {
        return Some("empty or relative component");
    }
    if component.chars().any(|c| {
        c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*')
    }) {
        return Some("character not allowed on Windows");
    }
    if component.ends_with('.') || component.ends_with(' ') {
        return Some("trailing dot or space (stripped by Windows)");
    }
    let stem = component.split('.').next().unwrap_or(component);
    if WINDOWS_RESERVED
        .iter()
        .any(|r| r.eq_ignore_ascii_case(stem))
    {
        return Some("reserved device name on Windows");
    }
    None
}

/// Checks every `/`-separated component of a relative path such as a bundle name.
pub fn check_relative(path: &str) -> Result<(), String> {
    if path.starts_with('/') {
        return Err(format!("{path:?}: absolute path"));
    }
    for component in path.split('/') {
        if let Some(problem) = component_problem(component) {
            return Err(format!("{path:?}: component {component:?}: {problem}"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_real_bundle_names() {
        for name in [
            "live2d/model/v2_25meiko_night",
            "scenario/unitstory/school-refusal-story-chapter",
            "sound/scenario/voice/nightcode_01_01",
        ] {
            assert_eq!(check_relative(name), Ok(()));
        }
    }

    #[test]
    fn rejects_names_windows_cannot_store() {
        for name in [
            "a/../b",
            "a//b",
            "/abs",
            "a/con",
            "a/Com1.txt",
            "a/b:c",
            "a/b\\c",
            "a/dot.",
            "a/space ",
        ] {
            assert!(check_relative(name).is_err(), "{name}");
        }
    }
}
