//! Artifact paths must stay outside the source tree.
//!
//! The guard reuses [`victory_core::policy::ensure_outside_repo`], which
//! rejects the compiled workspace root and any ancestor carrying the
//! `.victory-root` marker. Scenarios only name paths; this module validates
//! them before a runner or reporter may write anything.

use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use victory_core::policy::{PolicyError, ensure_outside_repo};

/// The file layout a scenario writes to, all outside the repository.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactSpec {
    /// JSONL file with one result object per run.
    pub result_file: PathBuf,
    /// Crash log passed to `victory_loader::crash::log_crashes_to`.
    pub crash_log: PathBuf,
    /// Directory for labelled diagnostic captures.
    pub captures_dir: PathBuf,
    /// Directory for other machine-readable evidence.
    pub evidence_dir: PathBuf,
}

/// Why an artifact path was refused.
#[derive(Debug, thiserror::Error)]
pub enum ArtifactError {
    #[error("artifact path {0} must be absolute")]
    NotAbsolute(PathBuf),
    #[error("artifact path {0} must not contain parent components")]
    ParentComponent(PathBuf),
    #[error("artifact paths must be distinct, {0} is repeated")]
    Duplicate(PathBuf),
    #[error("artifact path rejected: {0}")]
    Policy(#[from] PolicyError),
}

/// Validates one artifact path without creating it.
///
/// Absolute paths are required so a run cannot depend on the process working
/// directory. Parent components are refused even when the resulting path is
/// outside the repository, because their meaning can change after validation.
pub fn validate_path(path: &Path) -> Result<(), ArtifactError> {
    if !path.is_absolute() {
        return Err(ArtifactError::NotAbsolute(path.to_path_buf()));
    }
    if path.components().any(|c| c == Component::ParentDir) {
        return Err(ArtifactError::ParentComponent(path.to_path_buf()));
    }
    ensure_outside_repo(path)?;
    Ok(())
}

impl ArtifactSpec {
    /// Checks every path and that no two paths are the same.
    pub fn validate(&self) -> Result<(), ArtifactError> {
        let paths = [
            &self.result_file,
            &self.crash_log,
            &self.captures_dir,
            &self.evidence_dir,
        ];
        for path in paths {
            validate_path(path)?;
        }
        for (i, left) in paths.iter().enumerate() {
            for right in paths.iter().skip(i + 1) {
                if left == right {
                    return Err(ArtifactError::Duplicate((*left).clone()));
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(root: &Path) -> ArtifactSpec {
        ArtifactSpec {
            result_file: root.join("run.jsonl"),
            crash_log: root.join("crash.log"),
            captures_dir: root.join("captures"),
            evidence_dir: root.join("evidence"),
        }
    }

    #[test]
    fn accepts_paths_outside_the_repository() {
        let tmp = tempfile::tempdir().unwrap();
        spec(tmp.path()).validate().unwrap();
    }

    #[test]
    fn refuses_relative_paths() {
        let err = validate_path(Path::new("relative/run.jsonl")).unwrap_err();
        assert!(matches!(err, ArtifactError::NotAbsolute(_)), "{err}");
    }

    #[test]
    fn refuses_parent_components() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("..").join("elsewhere");
        let err = validate_path(&path).unwrap_err();
        assert!(matches!(err, ArtifactError::ParentComponent(_)), "{err}");
    }

    #[test]
    fn refuses_paths_inside_a_marked_tree() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join(".victory-root"), "").unwrap();
        let err = spec(tmp.path()).validate().unwrap_err();
        assert!(matches!(err, ArtifactError::Policy(_)), "{err}");
    }

    #[test]
    fn refuses_repeated_paths() {
        let tmp = tempfile::tempdir().unwrap();
        let mut layout = spec(tmp.path());
        layout.evidence_dir = layout.captures_dir.clone();
        let err = layout.validate().unwrap_err();
        assert!(matches!(err, ArtifactError::Duplicate(_)), "{err}");
    }
}
