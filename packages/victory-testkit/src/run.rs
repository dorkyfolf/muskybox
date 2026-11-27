//! Runner skeleton and crash recovery records.
//!
//! The single tester owns execution. [`Runner::plan`] only describes the
//! process, its environment and its isolated directories. It never spawns a
//! child, runs the game, or touches a GPU. [`Runner::recover`] records what a
//! crashed run reported, with paths already checked against the policy.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use crate::artifact::{ArtifactError, validate_path};
use crate::detect::StageSignal;
use crate::input::{self, InputError, KeyPress};
use crate::schema::{Checkpoint, CrashRecord, Scenario, SchemaError, TdrEvent};

/// Roots for isolated per-scenario data and cache directories.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunContext {
    data_root: PathBuf,
    cache_root: PathBuf,
}

impl RunContext {
    pub fn new(data_root: PathBuf, cache_root: PathBuf) -> Self {
        Self {
            data_root,
            cache_root,
        }
    }

    /// Returns `<data>/<id>` and `<cache>/<id>`, both validated outside the repo.
    pub fn subdirs(&self, scenario_id: &str) -> Result<(PathBuf, PathBuf), RunError> {
        let data = self.data_root.join(scenario_id);
        let cache = self.cache_root.join(scenario_id);
        validate_path(&data)?;
        validate_path(&cache)?;
        Ok((data, cache))
    }
}

/// A process description, ready for the tester to execute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunPlan {
    /// The runtime binary name.
    pub program: String,
    /// Arguments, normally `run --execute`.
    pub args: Vec<String>,
    /// Sorted environment for the child process.
    pub env: BTreeMap<String, String>,
    pub data_dir: PathBuf,
    pub cache_dir: PathBuf,
    /// Validated artifact layout.
    pub artifacts: crate::artifact::ArtifactSpec,
    /// Key presses in canonical order, mirrored from `VICTORY_KEYS`.
    pub input: Vec<KeyPress>,
    /// Hex RVAs added to `VICTORY_WATCH`.
    pub watch_rvas: Vec<String>,
}

/// Why a plan or recovery record could not be built.
#[derive(Debug, thiserror::Error)]
pub enum RunError {
    #[error("artifact policy: {0}")]
    Artifact(#[from] ArtifactError),
    #[error("scenario schema: {0}")]
    Schema(#[from] SchemaError),
    #[error("input script: {0}")]
    Input(#[from] InputError),
}

/// Builds plans and recovery records without executing anything.
#[derive(Debug, Default, Clone, Copy)]
pub struct Runner;

impl Runner {
    /// Compiles a scenario into a deterministic run plan.
    pub fn plan(&self, scenario: &Scenario, context: &RunContext) -> Result<RunPlan, RunError> {
        scenario.artifacts.validate()?;
        Checkpoint::ordered(&scenario.checkpoints)?;
        let (data_dir, cache_dir) = context.subdirs(&scenario.id)?;

        let script = input::compile(&scenario.key_presses())?;
        let mut env = scenario.env.clone();
        env.entry("VICTORY_KEYS".into())
            .or_insert_with(|| script.env_value().to_owned());
        let watch_rvas: Vec<String> = scenario
            .checkpoints
            .iter()
            .flat_map(|checkpoint| &checkpoint.signals)
            .filter_map(|signal| match signal {
                StageSignal::Watch { rva, .. } => Some(format!("{rva:x}")),
                _ => None,
            })
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        if !watch_rvas.is_empty() {
            env.entry("VICTORY_WATCH".into())
                .or_insert_with(|| watch_rvas.join(","));
        }

        Ok(RunPlan {
            program: "victory".into(),
            args: vec!["run".into(), "--execute".into()],
            env,
            data_dir,
            cache_dir,
            artifacts: scenario.artifacts.clone(),
            input: script.presses().to_vec(),
            watch_rvas,
        })
    }

    /// Records a crashed run with its exit status, log and TDR events.
    pub fn recover(
        &self,
        scenario: &Scenario,
        exit_code: Option<i32>,
        tdrs: Vec<TdrEvent>,
        at_ms: u64,
    ) -> Result<CrashRecord, RunError> {
        scenario.artifacts.validate()?;
        Ok(CrashRecord {
            exit_code,
            crash_log: scenario.artifacts.crash_log.clone(),
            artifact_dir: scenario.artifacts.evidence_dir.clone(),
            tdrs,
            at_ms,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

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
signals = [{{ kind = "watch", rva = 4660, value = 7 }}]

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
    fn plans_a_deterministic_process() {
        let tmp = tempfile::tempdir().unwrap();
        let scenario = Scenario::from_toml(&toml_text(tmp.path())).unwrap();
        let context = RunContext::new(tmp.path().join("data"), tmp.path().join("cache"));
        let plan = Runner.plan(&scenario, &context).unwrap();
        assert_eq!(plan.program, "victory");
        assert_eq!(plan.args, vec!["run", "--execute"]);
        assert_eq!(plan.env["VICTORY_KEYS"], "2.000:Enter");
        assert_eq!(plan.env["VICTORY_WATCH"], "1234");
        assert_eq!(plan.input, scenario.key_presses());
        assert_eq!(plan.watch_rvas, vec!["1234"]);
        assert_eq!(plan.data_dir, tmp.path().join("data").join("boot_to_menu"));
        assert_eq!(
            plan.cache_dir,
            tmp.path().join("cache").join("boot_to_menu")
        );
    }

    #[test]
    fn plan_refuses_invalid_checkpoints() {
        let tmp = tempfile::tempdir().unwrap();
        let text = toml_text(tmp.path()).replace("order = 2", "order = 1");
        let scenario = Scenario::from_toml(&text).unwrap();
        let context = RunContext::new(tmp.path().join("data"), tmp.path().join("cache"));
        assert!(matches!(
            Runner.plan(&scenario, &context),
            Err(RunError::Schema(SchemaError::DuplicateOrder(1)))
        ));
    }

    #[test]
    fn recovers_crash_information() {
        let tmp = tempfile::tempdir().unwrap();
        let scenario = Scenario::from_toml(&toml_text(tmp.path())).unwrap();
        let tdrs = vec![TdrEvent {
            at_ms: 4200,
            device: "WARP".into(),
            code: Some(-1),
        }];
        let crash = Runner
            .recover(&scenario, Some(-1073741819), tdrs.clone(), 5000)
            .unwrap();
        assert_eq!(crash.exit_code, Some(-1073741819));
        assert_eq!(crash.crash_log, scenario.artifacts.crash_log);
        assert_eq!(crash.artifact_dir, scenario.artifacts.evidence_dir);
        assert_eq!(crash.tdrs, tdrs);
        assert_eq!(crash.at_ms, 5000);
    }
}
