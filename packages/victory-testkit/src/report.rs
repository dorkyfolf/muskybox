//! Machine-readable result output.
//!
//! One JSON object per run is appended to the scenario's JSONL file. The path
//! is validated before any write, so a result can never land in the source
//! tree. Serialization is kept separate so it can be tested without files.

use std::io::Write;
use std::path::Path;

use crate::artifact::{ArtifactError, validate_path};
use crate::schema::ScenarioResult;

/// Why a result could not be serialized or written.
#[derive(Debug, thiserror::Error)]
pub enum ReportError {
    #[error("artifact policy: {0}")]
    Artifact(#[from] ArtifactError),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

/// Serializes one result as a single JSON value, without a trailing newline.
pub fn to_json_line(result: &ScenarioResult) -> Result<String, ReportError> {
    Ok(serde_json::to_string(result)?)
}

/// Reads one result back from a JSON line, for evidence checks.
pub fn from_json_line(line: &str) -> Result<ScenarioResult, ReportError> {
    Ok(serde_json::from_str(line)?)
}

/// Appends one result line to `path` after validating the artifact path.
pub fn append(path: &Path, result: &ScenarioResult) -> Result<(), ReportError> {
    validate_path(path)?;
    let line = to_json_line(result)?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(file, "{line}")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detect::Stage;
    use crate::metrics::Metrics;
    use crate::schema::RunStatus;

    fn result() -> ScenarioResult {
        ScenarioResult {
            scenario_id: "boot_to_menu".into(),
            status: RunStatus::Passed,
            stage: Stage::Menu,
            metrics: Metrics {
                average_fps: 80.0,
                fps_1_low: 50.0,
                frame_time_ms: 12.5,
                copy_bytes: 1000,
                gpu_time_ms: 5.0,
                frames: 4,
            },
            transitions: Vec::new(),
            crashes: Vec::new(),
            tdrs: Vec::new(),
            exit_code: Some(0),
        }
    }

    #[test]
    fn serializes_a_complete_result() {
        let line = to_json_line(&result()).unwrap();
        assert!(!line.contains('\n'));
        assert!(line.contains("\"scenario_id\":\"boot_to_menu\""));
        assert!(line.contains("\"status\":\"passed\""));
        assert!(line.contains("\"stage\":\"menu\""));
        assert!(line.contains("\"fps_1_low\":50.0"));
        assert_eq!(from_json_line(&line).unwrap(), result());
    }

    #[test]
    fn refuses_to_write_inside_the_repository() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("result.jsonl");
        assert!(matches!(
            append(&path, &result()),
            Err(ReportError::Artifact(_))
        ));
        assert!(!path.exists());
    }
}
