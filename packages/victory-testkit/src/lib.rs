//! Reproducible, asset-free test harness primitives for the VItoriaBr runtime.
//!
//! This crate is source only. It never launches the title, touches the GPU,
//! reads game files, or stores game data. The single tester owns execution and
//! supplies the game paths at run time. Every derived path (cache, capture,
//! crash log and result file) is checked against the asset policy before use.
//!
//! The modules follow the harness scout design:
//!
//! - [`schema`] holds the scenario and result records;
//! - [`detect`] confirms stages from several independent signals;
//! - [`input`] compiles deterministic `VICTORY_KEYS` scripts;
//! - [`metrics`] calculates frame, copy and GPU time metrics;
//! - [`artifact`] enforces paths outside the source tree;
//! - [`run`] describes the process plan and crash recovery records;
//! - [`report`] writes machine-readable results.

pub mod artifact;
pub mod detect;
pub mod input;
pub mod metrics;
pub mod report;
pub mod run;
pub mod schema;
