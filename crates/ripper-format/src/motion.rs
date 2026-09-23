//! `sse-motion` v1: a lossless dump of one Unity `AnimationClip` as used by Live2D story playback.
//!
//! The clip's StreamedClip keys are kept as the original cubic coefficients rather than
//! being resampled or converted to motion3 (which loses the zero-tangent cubic shape).
//! All floats are `f32` and serialise in their shortest round-trip form, so reading a
//! document back yields bit-identical values.

use serde::{Deserialize, Serialize};

pub const FORMAT: &str = "sse-motion";
pub const VERSION: u32 = 1;

/// `crc32("Value")`: the attribute of every Cubism parameter binding.
pub const ATTR_VALUE: u32 = 3_702_945_584;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SseMotion {
    pub format: String,
    pub version: u32,
    /// `AnimationClip.m_Name`.
    pub name: String,
    pub sample_rate: f32,
    /// `m_MuscleClip.m_StartTime` / `m_StopTime` / `m_LoopTime`.
    pub start_time: f32,
    pub stop_time: f32,
    pub loop_time: bool,
    /// `Live2DBuildMotionMetaData.FadeInTime` / `FadeOutTime`, when the metadata object was found.
    pub fade: Option<Fade>,
    pub curves: Vec<Curve>,
    pub events: Vec<Event>,
    pub source: Source,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Fade {
    #[serde(rename = "in")]
    pub fade_in: f32,
    #[serde(rename = "out")]
    pub fade_out: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Curve {
    pub binding: Binding,
    #[serde(flatten)]
    pub data: CurveData,
}

/// One entry of `m_ClipBindingConstant.genericBindings`, plus the names recovered from its hashes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Binding {
    /// `crc32("Parameters/<id>")` or `crc32("Parts/<id>")`; 0 for component-level curves.
    pub path_hash: u32,
    pub attr_hash: u32,
    pub type_id: i32,
    pub custom_type: u8,
    /// Filled when the path hash matched a known parameter or part id; `None` keeps the hash authoritative.
    pub resolved: Option<ResolvedBinding>,
    /// Name of the attribute hash when known (`Value`, `Opacity`, `EyeOpening`, `MouthOpening`).
    pub attr: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedBinding {
    pub target: BindingTarget,
    pub id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BindingTarget {
    Parameter,
    Part,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum CurveData {
    /// Piecewise cubic: from `segments[i].t` until the next segment starts,
    /// `value(t) = ((a·dt + b)·dt + c)·dt + d` with `dt = t − segments[i].t` and `coeff = [a, b, c, d]`.
    /// The first segment starts at `-f32::MAX`, exactly as stored by Unity.
    Streamed {
        segments: Vec<Segment>,
    },
    /// Uniformly sampled values starting at `begin_time`.
    Dense {
        begin_time: f32,
        sample_rate: f32,
        samples: Vec<f32>,
    },
    Constant {
        value: f32,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Segment {
    pub t: f32,
    pub coeff: [f32; 4],
}

/// One `AnimationEvent`, e.g. `OnLive2DInvokeUserData("eyeblink,0.2,0.79,0.24")`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    pub time: f32,
    pub function: String,
    pub data: String,
    pub float_parameter: f32,
    pub int_parameter: i32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Source {
    pub bundle: String,
    pub path_id: i64,
    /// Container path of the clip inside the bundle, when it has one.
    pub container: Option<String>,
}

impl CurveData {
    /// Evaluates the curve the way Unity's StreamedClip / DenseClip / ConstantClip do.
    pub fn evaluate(&self, time: f32) -> f32 {
        match self {
            Self::Constant { value } => *value,
            Self::Streamed { segments } => {
                let index = segments.partition_point(|s| s.t <= time).saturating_sub(1);
                let Some(segment) = segments.get(index) else {
                    return 0.0;
                };
                let [a, b, c, d] = segment.coeff;
                let dt = time - segment.t;
                ((a * dt + b) * dt + c) * dt + d
            }
            Self::Dense {
                begin_time,
                sample_rate,
                samples,
            } => {
                if samples.is_empty() {
                    return 0.0;
                }
                let position = ((time - begin_time) * sample_rate).max(0.0);
                let last = samples.len() - 1;
                let lower = (position.floor() as usize).min(last);
                let upper = (lower + 1).min(last);
                let fraction = position - lower as f32;
                samples[lower] + (samples[upper] - samples[lower]) * fraction.clamp(0.0, 1.0)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streamed_evaluation_uses_the_segment_that_starts_at_or_before_t() {
        let curve = CurveData::Streamed {
            segments: vec![
                Segment {
                    t: -f32::MAX,
                    coeff: [0.0, 0.0, 0.0, 1.0],
                },
                Segment {
                    t: 0.0,
                    coeff: [0.0, 0.0, 2.0, 1.0],
                },
                Segment {
                    t: 1.0,
                    coeff: [1.0, 0.0, 0.0, 3.0],
                },
            ],
        };
        assert_eq!(curve.evaluate(-1.0), 1.0);
        assert_eq!(curve.evaluate(0.5), 2.0);
        assert_eq!(curve.evaluate(1.5), 3.125);
    }

    #[test]
    fn f32_values_round_trip_bit_exactly_through_json() {
        let values = [
            0.1_f32,
            -24.635_777,
            16.000_938,
            f32::MIN_POSITIVE,
            -f32::MAX,
            1.0e-45,
        ];
        let curve = CurveData::Streamed {
            segments: values
                .iter()
                .map(|&v| Segment {
                    t: v,
                    coeff: [v, v, v, v],
                })
                .collect(),
        };
        let json = serde_json::to_string(&curve).unwrap();
        let back: CurveData = serde_json::from_str(&json).unwrap();
        let CurveData::Streamed { segments } = back else {
            panic!("kind changed")
        };
        for (segment, value) in segments.iter().zip(values) {
            assert_eq!(segment.t.to_bits(), value.to_bits());
            assert!(segment.coeff.iter().all(|c| c.to_bits() == value.to_bits()));
        }
    }
}
