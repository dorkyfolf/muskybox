//! Metrics calculated from exported frame, copy and GPU time samples.
//!
//! The runtime exposes these values through tracing lines and
//! `victory_gpu::copystat`; this module consumes the samples and never reads a
//! GPU counter itself. Keeping the calculation here makes it portable and
//! testable without the title or a device.

use serde::{Deserialize, Serialize};

/// One frame's measured costs.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrameSample {
    /// Wall time between presents, in milliseconds.
    pub frame_time_ms: f64,
    /// Bytes copied by the host during this frame.
    pub copy_bytes: u64,
    /// GPU execution time for this frame, in milliseconds.
    pub gpu_time_ms: f64,
}

/// Aggregate metrics for one measurement window.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Metrics {
    /// 1000 divided by the mean frame time.
    pub average_fps: f64,
    /// 1000 divided by the p99 frame time.
    pub fps_1_low: f64,
    /// Mean frame time in milliseconds.
    pub frame_time_ms: f64,
    /// Total bytes copied in the window.
    pub copy_bytes: u64,
    /// Mean GPU time per frame, in milliseconds.
    pub gpu_time_ms: f64,
    /// Number of samples in the window.
    pub frames: usize,
}

impl Metrics {
    /// Calculates metrics from samples, or `None` for an invalid window.
    ///
    /// Frame times must be finite and positive. The p99 index on ascending
    /// frame times is `ceil(0.99 * n) - 1`, clamped into the slice, so short
    /// windows use the slowest frame as their 1% low. This extension is
    /// documented because a ten-second window at 30 FPS has only 300 frames.
    pub fn from_samples(samples: &[FrameSample]) -> Option<Self> {
        if samples.is_empty()
            || samples
                .iter()
                .any(|s| !s.frame_time_ms.is_finite() || s.frame_time_ms <= 0.0)
        {
            return None;
        }
        let n = samples.len();
        let mut times: Vec<f64> = samples.iter().map(|s| s.frame_time_ms).collect();
        times.sort_by(|a, b| a.total_cmp(b));
        let p99 = (99 * n).div_ceil(100);
        let index = p99.max(1).min(n) - 1;
        let mean_time = samples.iter().map(|s| s.frame_time_ms).sum::<f64>() / n as f64;
        let mean_gpu = samples.iter().map(|s| s.gpu_time_ms).sum::<f64>() / n as f64;
        Some(Self {
            average_fps: 1000.0 / mean_time,
            fps_1_low: 1000.0 / times[index],
            frame_time_ms: mean_time,
            copy_bytes: samples.iter().map(|s| s.copy_bytes).sum(),
            gpu_time_ms: mean_gpu,
            frames: n,
        })
    }

    /// Calculates metrics from frame times alone, with zero copy and GPU time.
    pub fn from_frame_times_ms(times: &[f64]) -> Option<Self> {
        let samples: Vec<FrameSample> = times
            .iter()
            .map(|&frame_time_ms| FrameSample {
                frame_time_ms,
                copy_bytes: 0,
                gpu_time_ms: 0.0,
            })
            .collect();
        Self::from_samples(&samples)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-9, "{a} is not close to {b}");
    }

    fn samples() -> Vec<FrameSample> {
        [10.0, 10.0, 10.0, 20.0]
            .into_iter()
            .zip([100, 200, 300, 400])
            .zip([2.0, 4.0, 6.0, 8.0])
            .map(|((frame_time_ms, copy_bytes), gpu_time_ms)| FrameSample {
                frame_time_ms,
                copy_bytes,
                gpu_time_ms,
            })
            .collect()
    }

    #[test]
    fn calculates_all_metrics() {
        let metrics = Metrics::from_samples(&samples()).unwrap();
        close(metrics.average_fps, 80.0);
        close(metrics.fps_1_low, 50.0);
        close(metrics.frame_time_ms, 12.5);
        assert_eq!(metrics.copy_bytes, 1000);
        close(metrics.gpu_time_ms, 5.0);
        assert_eq!(metrics.frames, 4);
    }

    #[test]
    fn uses_the_slowest_frame_for_short_windows() {
        let metrics = Metrics::from_frame_times_ms(&[16.0, 33.0]).unwrap();
        close(metrics.fps_1_low, 1000.0 / 33.0);
        assert_eq!(metrics.copy_bytes, 0);
        close(metrics.gpu_time_ms, 0.0);
    }

    #[test]
    fn rejects_invalid_windows() {
        assert!(Metrics::from_samples(&[]).is_none());
        assert!(Metrics::from_frame_times_ms(&[16.0, 0.0]).is_none());
        assert!(Metrics::from_frame_times_ms(&[f64::NAN]).is_none());
    }
}
