//! Stage detection from several independent signals.
//!
//! A screenshot is evidence, not proof. The runtime already emits tracing
//! lines, draw counts, `VICTORY_WATCH` values, title debug output and save-file
//! changes. A stage is confirmed only when a quorum of distinct non-screenshot
//! signals for that stage have been observed. This keeps a black or cached
//! frame from being mistaken for a menu or map transition.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

/// Lifecycle stages of the title under test.
#[derive(
    Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    #[default]
    Unknown,
    Boot,
    Menu,
    Map,
    Mission,
}

/// A source of stage evidence available from the runtime.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StageSignal {
    /// A `tracing` line whose target and message contain these strings.
    Trace { target: String, contains: String },
    /// Draw calls since the last present, within an inclusive range.
    DrawCount { min: u64, max: u64 },
    /// A `VICTORY_WATCH` address whose u32 became `value`.
    Watch { rva: u32, value: u32 },
    /// Title debug output containing this string.
    DebugOutput { contains: String },
    /// A save file created or changed at this relative path.
    SaveFile { path: String },
    /// A labelled capture. Recorded as evidence, never as confirmation.
    Screenshot { label: String },
}

impl StageSignal {
    /// Whether this signal may confirm a stage.
    pub fn confirms(&self) -> bool {
        !matches!(self, Self::Screenshot { .. })
    }

    /// The source kind, used so two values from one source count once.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Trace { .. } => "trace",
            Self::DrawCount { .. } => "draw_count",
            Self::Watch { .. } => "watch",
            Self::DebugOutput { .. } => "debug_output",
            Self::SaveFile { .. } => "save_file",
            Self::Screenshot { .. } => "screenshot",
        }
    }
}

/// One signal observed at a point in the scenario timeline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Observation {
    pub at_frame: u64,
    pub at_ms: u64,
    pub stage: Stage,
    pub signal: StageSignal,
}

/// Confirms stages from observations collected so far.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageDetector {
    quorum: usize,
    confirmed: Vec<Stage>,
}

impl Default for StageDetector {
    fn default() -> Self {
        Self::new()
    }
}

impl StageDetector {
    /// Creates a detector needing two independent signals per stage.
    pub fn new() -> Self {
        Self {
            quorum: 2,
            confirmed: Vec::new(),
        }
    }

    /// Creates a detector with a different confirmation quorum.
    pub fn with_quorum(quorum: usize) -> Self {
        Self {
            quorum,
            confirmed: Vec::new(),
        }
    }

    pub fn quorum(&self) -> usize {
        self.quorum
    }

    /// The last confirmed stage, or `Unknown` before the first confirmation.
    pub fn stage(&self) -> Stage {
        self.confirmed.last().copied().unwrap_or_default()
    }

    /// Confirms a stage from a set of observations without changing state.
    ///
    /// Distinct signal kinds are counted per stage. When several stages
    /// qualify, the one with the most recent frame wins, with the total count
    /// as a tie breaker. Screenshots are ignored by the count.
    pub fn confirm(observations: &[Observation], quorum: usize) -> Option<Stage> {
        if quorum == 0 {
            return None;
        }
        let mut best: Option<(Stage, u64, usize)> = None;
        for stage in [
            Stage::Unknown,
            Stage::Boot,
            Stage::Menu,
            Stage::Map,
            Stage::Mission,
        ] {
            let mut kinds = BTreeSet::new();
            let mut last_frame = 0;
            let mut count = 0;
            for observation in observations.iter().filter(|o| o.stage == stage) {
                if observation.signal.confirms() && kinds.insert(observation.signal.kind()) {
                    count += 1;
                }
                last_frame = last_frame.max(observation.at_frame);
            }
            if count >= quorum {
                let better = best.is_none_or(|(_, frame, n)| (last_frame, count) > (frame, n));
                if better {
                    best = Some((stage, last_frame, count));
                }
            }
        }
        best.map(|(stage, _, _)| stage)
    }

    /// Feeds observations in timeline order and returns newly confirmed stages.
    pub fn observe(&mut self, observations: &[Observation]) -> Vec<Stage> {
        let mut seen = Vec::new();
        let mut fresh = Vec::new();
        for observation in observations {
            seen.push(observation.clone());
            let Some(stage) = Self::confirm(&seen, self.quorum) else {
                continue;
            };
            if self.confirmed.last() != Some(&stage) {
                self.confirmed.push(stage);
                fresh.push(stage);
            }
        }
        fresh
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observation(frame: u64, stage: Stage, signal: StageSignal) -> Observation {
        Observation {
            at_frame: frame,
            at_ms: frame * 16,
            stage,
            signal,
        }
    }

    fn trace(target: &str, contains: &str) -> StageSignal {
        StageSignal::Trace {
            target: target.into(),
            contains: contains.into(),
        }
    }

    #[test]
    fn screenshots_alone_never_confirm() {
        let observations = [
            observation(
                1,
                Stage::Menu,
                StageSignal::Screenshot {
                    label: "menu".into(),
                },
            ),
            observation(
                2,
                Stage::Menu,
                StageSignal::Screenshot {
                    label: "menu2".into(),
                },
            ),
        ];
        assert_eq!(StageDetector::confirm(&observations, 2), None);
        let mut detector = StageDetector::new();
        assert!(detector.observe(&observations).is_empty());
        assert_eq!(detector.stage(), Stage::Unknown);
    }

    #[test]
    fn distinct_signals_confirm_a_stage() {
        let trace = trace("victory_gpu::frame", "frame interval");
        let draw = StageSignal::DrawCount { min: 40, max: 400 };
        let shot = StageSignal::Screenshot {
            label: "menu".into(),
        };
        let observations = [
            observation(1, Stage::Menu, shot),
            observation(2, Stage::Menu, trace),
            observation(3, Stage::Menu, draw),
        ];
        assert_eq!(StageDetector::confirm(&observations, 2), Some(Stage::Menu));
    }

    #[test]
    fn two_values_from_one_source_are_not_a_quorum() {
        let a = StageSignal::Watch {
            rva: 0x1234,
            value: 1,
        };
        let b = StageSignal::Watch {
            rva: 0x1234,
            value: 2,
        };
        let observations = [observation(1, Stage::Map, a), observation(2, Stage::Map, b)];
        assert_eq!(StageDetector::confirm(&observations, 2), None);
    }

    #[test]
    fn observe_reports_transitions_in_order() {
        let observations = [
            observation(
                1,
                Stage::Menu,
                StageSignal::Screenshot {
                    label: "menu".into(),
                },
            ),
            observation(
                2,
                Stage::Menu,
                trace("victory_gpu::frame", "frame interval"),
            ),
            observation(3, Stage::Menu, StageSignal::DrawCount { min: 1, max: 1000 }),
            observation(
                4,
                Stage::Map,
                StageSignal::Watch {
                    rva: 0x50,
                    value: 7,
                },
            ),
            observation(
                5,
                Stage::Map,
                StageSignal::SaveFile {
                    path: "save/slot1".into(),
                },
            ),
        ];
        let mut detector = StageDetector::new();
        assert_eq!(
            detector.observe(&observations),
            vec![Stage::Menu, Stage::Map]
        );
        assert_eq!(detector.stage(), Stage::Map);
    }

    #[test]
    fn quorum_is_configurable() {
        let observations = [
            observation(1, Stage::Boot, trace("victory_loader", "loaded")),
            observation(
                2,
                Stage::Boot,
                StageSignal::DebugOutput {
                    contains: "boot".into(),
                },
            ),
        ];
        assert_eq!(StageDetector::confirm(&observations, 3), None);
        assert_eq!(
            StageDetector::with_quorum(2).quorum(),
            2,
            "custom detector keeps its quorum"
        );
    }
}
