//! Scenario and result records shared by the builders and the tester.
//!
//! Scenarios are TOML files without game data. Results are JSON objects, one
//! per run, appended to a JSONL file outside the repository.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::artifact::ArtifactSpec;
use crate::detect::{Stage, StageSignal};
use crate::input::KeyPress;
use crate::metrics::Metrics;

/// A duration in whole milliseconds, as written in scenario files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TimeoutMs(pub u64);

impl TimeoutMs {
    pub fn millis(self) -> u64 {
        self.0
    }
}

/// Which runtime build a scenario targets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuildSpec {
    /// Cargo profile, normally `diag` so the diagnostics feature is available.
    pub profile: String,
    /// Extra Cargo features, for example `diagnostics`.
    #[serde(default)]
    pub features: Vec<String>,
    /// Identity returned by `victory_game::identify`, empty for known builds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_id: Option<String>,
}

/// One deterministic action in a scenario timeline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Step {
    /// Start the runtime with `victory run --execute`.
    Launch,
    /// Press a documented key at an offset from input start.
    Key { key: String, at_ms: u64 },
    /// Wait until the runtime has begun this many frames.
    WaitFrames { frames: u64 },
    /// Wait a fixed number of milliseconds.
    WaitMs { ms: u64 },
    /// Assert that the ordered checkpoints were reached.
    AssertCheckpoints,
    /// Ask the runtime for a labelled diagnostic capture.
    Capture { label: String },
    /// Stop the title with a documented key, then collect artifacts.
    Terminate,
}

/// A named stage assertion in the scenario timeline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Checkpoint {
    /// Stable name used in the result and in transition evidence.
    pub id: String,
    /// Declaration order in the timeline.
    pub order: u32,
    /// Stage expected at this checkpoint.
    pub stage: Stage,
    /// Extra signals recorded for this checkpoint.
    #[serde(default)]
    pub signals: Vec<StageSignal>,
}

/// Why a scenario could not be read or ordered.
#[derive(Debug, thiserror::Error)]
pub enum SchemaError {
    #[error("cannot read scenario: {0}")]
    Read(#[from] toml::de::Error),
    #[error("cannot write scenario: {0}")]
    Write(#[from] toml::ser::Error),
    #[error("duplicate checkpoint order {0}")]
    DuplicateOrder(u32),
    #[error("duplicate checkpoint id {0}")]
    DuplicateId(String),
}

impl Checkpoint {
    /// Returns checkpoints sorted by order, rejecting duplicate orders and ids.
    pub fn ordered(checkpoints: &[Checkpoint]) -> Result<Vec<Checkpoint>, SchemaError> {
        let mut sorted = checkpoints.to_vec();
        sorted.sort_by_key(|c| c.order);
        let mut orders = std::collections::BTreeSet::new();
        let mut ids = std::collections::BTreeSet::new();
        for checkpoint in &sorted {
            if !orders.insert(checkpoint.order) {
                return Err(SchemaError::DuplicateOrder(checkpoint.order));
            }
            if !ids.insert(checkpoint.id.clone()) {
                return Err(SchemaError::DuplicateId(checkpoint.id.clone()));
            }
        }
        Ok(sorted)
    }

    /// Returns checkpoint ids in timeline order.
    pub fn ordered_ids(checkpoints: &[Checkpoint]) -> Result<Vec<String>, SchemaError> {
        Ok(Self::ordered(checkpoints)?
            .into_iter()
            .map(|c| c.id)
            .collect())
    }
}

/// A reproducible scenario, stored as TOML outside the game tree.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    /// Unique scenario name, also used for isolated data and cache subdirs.
    pub id: String,
    pub build: BuildSpec,
    #[serde(default)]
    pub steps: Vec<Step>,
    #[serde(default)]
    pub checkpoints: Vec<Checkpoint>,
    pub timeout: TimeoutMs,
    /// Environment passed to the runtime, including scripted keys and watches.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// Validated output layout for this scenario.
    pub artifacts: ArtifactSpec,
}

impl Scenario {
    pub fn from_toml(text: &str) -> Result<Self, SchemaError> {
        Ok(toml::from_str(text)?)
    }

    pub fn to_toml(&self) -> Result<String, SchemaError> {
        Ok(toml::to_string_pretty(self)?)
    }

    /// Key steps in declaration order, before canonical compilation.
    pub fn key_presses(&self) -> Vec<KeyPress> {
        self.steps
            .iter()
            .filter_map(|step| match step {
                Step::Key { key, at_ms } => Some(KeyPress {
                    at_ms: *at_ms,
                    key: key.clone(),
                }),
                _ => None,
            })
            .collect()
    }
}

/// Overall outcome of one scenario run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Passed,
    Failed,
    TimedOut,
    Crashed,
    Tdr,
    Skipped,
}

/// A device timeout detected and reset by the host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TdrEvent {
    /// Milliseconds since input start.
    pub at_ms: u64,
    /// Device identifier reported by the host.
    pub device: String,
    /// Host status code, when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<i32>,
}

/// A crash recovered from the runtime crash log and process status.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CrashRecord {
    /// Process exit code, when the runtime exited.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    /// Crash log written by `victory_loader::crash::log_crashes_to`.
    pub crash_log: std::path::PathBuf,
    /// Artifact directory for this run, outside the repository.
    pub artifact_dir: std::path::PathBuf,
    /// TDR events observed before the crash.
    #[serde(default)]
    pub tdrs: Vec<TdrEvent>,
    /// Milliseconds since input start.
    pub at_ms: u64,
}

/// A confirmed stage change with the signals that justify it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Transition {
    pub from: Stage,
    pub to: Stage,
    pub at_frame: u64,
    pub at_ms: u64,
    pub signals: Vec<StageSignal>,
}

/// Machine-readable result, one JSON object per run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScenarioResult {
    pub scenario_id: String,
    pub status: RunStatus,
    /// Last confirmed stage.
    pub stage: Stage,
    #[serde(default)]
    pub metrics: Metrics,
    #[serde(default)]
    pub transitions: Vec<Transition>,
    #[serde(default)]
    pub crashes: Vec<CrashRecord>,
    #[serde(default)]
    pub tdrs: Vec<TdrEvent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn toml_text(root: &Path) -> String {
        format!(
            r#"id = "boot_to_menu"
timeout = 10000

[build]
profile = "diag"
features = ["diagnostics"]

[env]
VICTORY_WATCH = "1234"

[[steps]]
kind = "key"
key = "Enter"
at_ms = 2000

[[steps]]
kind = "wait_frames"
frames = 30

[[checkpoints]]
id = "boot"
order = 1
stage = "boot"

[[checkpoints]]
id = "menu"
order = 2
stage = "menu"

[artifacts]
result_file = '{}'
crash_log = '{}'
captures_dir = '{}'
evidence_dir = '{}'
"#,
            root.join("result.jsonl").display(),
            root.join("crash.log").display(),
            root.join("captures").display(),
            root.join("evidence").display(),
        )
    }

    #[test]
    fn parses_a_scenario() {
        let tmp = tempfile::tempdir().unwrap();
        let scenario = Scenario::from_toml(&toml_text(tmp.path())).unwrap();
        assert_eq!(scenario.id, "boot_to_menu");
        assert_eq!(scenario.build.profile, "diag");
        assert_eq!(scenario.timeout, TimeoutMs(10000));
        assert_eq!(scenario.env["VICTORY_WATCH"], "1234");
        assert_eq!(scenario.key_presses().len(), 1);
        assert_eq!(
            scenario.artifacts.result_file,
            tmp.path().join("result.jsonl")
        );
    }

    #[test]
    fn round_trips_a_scenario() {
        let tmp = tempfile::tempdir().unwrap();
        let original = Scenario::from_toml(&toml_text(tmp.path())).unwrap();
        let text = original.to_toml().unwrap();
        assert_eq!(Scenario::from_toml(&text).unwrap(), original);
    }

    #[test]
    fn refuses_unknown_fields() {
        let tmp = tempfile::tempdir().unwrap();
        let text =
            toml_text(tmp.path()).replace("timeout = 10000", "timeout = 10000\nunexpected = 1");
        assert!(matches!(
            Scenario::from_toml(&text),
            Err(SchemaError::Read(_))
        ));
    }

    #[test]
    fn orders_checkpoints() {
        let checkpoints = [
            Checkpoint {
                id: "menu".into(),
                order: 2,
                stage: Stage::Menu,
                signals: Vec::new(),
            },
            Checkpoint {
                id: "boot".into(),
                order: 1,
                stage: Stage::Boot,
                signals: Vec::new(),
            },
        ];
        assert_eq!(
            Checkpoint::ordered_ids(&checkpoints).unwrap(),
            vec!["boot", "menu"]
        );
        let mut duplicate = checkpoints.to_vec();
        duplicate[1].order = 2;
        assert!(matches!(
            Checkpoint::ordered(&duplicate),
            Err(SchemaError::DuplicateOrder(2))
        ));
    }
}
