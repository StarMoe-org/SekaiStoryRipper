//! `AnimationClip` typetree → `sse-motion` (ADR-0007), keeping StreamedClip coefficients as stored.
//!
//! Curve order follows Unity's muscle clip: streamed curves, then dense curves, then constant
//! curves. Bindings map onto them in order; a `Transform` binding (`typeID` 4) covers several
//! consecutive curves — position 3, rotation (quaternion) 4, scale 3, Euler angles 3 — which
//! become one curve each, named `m_LocalPosition.x` … with `attrHash = crc32(name)`.

use std::collections::HashMap;

use ripper_format::motion::{
    self, Binding, BindingTarget, Curve, CurveData, Event, Fade, ResolvedBinding, Segment, Source,
    SseMotion,
};
use serde_json::Value;

use crate::{ConvertError, Result, malformed};

/// Maps binding path hashes (`crc32("Parameters/<id>")`, `crc32("Parts/<id>")`) to ids.
#[derive(Debug, Default, Clone)]
pub struct BindingNames {
    by_path: HashMap<u32, ResolvedBinding>,
}

impl BindingNames {
    pub fn add_model(&mut self, parameters: &[String], parts: &[String]) {
        let entries = parameters
            .iter()
            .map(|id| (BindingTarget::Parameter, "Parameters/", id))
            .chain(parts.iter().map(|id| (BindingTarget::Part, "Parts/", id)));
        for (target, prefix, id) in entries {
            let hash = crc32fast::hash(format!("{prefix}{id}").as_bytes());
            self.by_path.entry(hash).or_insert_with(|| ResolvedBinding {
                target,
                id: id.clone(),
            });
        }
    }

    pub fn is_known(&self, path_hash: u32) -> bool {
        self.by_path.contains_key(&path_hash)
    }

    pub fn resolve(&self, path_hash: u32) -> Option<ResolvedBinding> {
        self.by_path.get(&path_hash).cloned()
    }
}

/// Names of the attribute hashes seen on Sekai Live2D clips.
pub fn attribute_name(hash: u32) -> Option<&'static str> {
    ["Value", "Opacity", "EyeOpening", "MouthOpening"]
        .into_iter()
        .find(|name| crc32fast::hash(name.as_bytes()) == hash)
}

/// One StreamedClip frame: all keys that start a new segment at `time`.
#[derive(Debug, Clone, PartialEq)]
pub struct StreamedFrame {
    pub time: f32,
    pub keys: Vec<(u32, [f32; 4])>,
}

/// Decodes `m_StreamedClip.data`: per frame `[time f32][keyCount i32]` then `keyCount` ×
/// `[curveIndex i32][coeff f32 × 4]`, all little-endian 32-bit words.
pub fn decode_streamed(words: &[u32]) -> Result<Vec<StreamedFrame>> {
    let mut frames = Vec::new();
    let mut stream = words.iter().copied().peekable();
    let next = |stream: &mut std::iter::Peekable<_>, what: &str| -> Result<u32> {
        Iterator::next(stream)
            .ok_or_else(|| malformed("StreamedClip", format!("truncated while reading {what}")))
    };
    while stream.peek().is_some() {
        let time = f32::from_bits(next(&mut stream, "frame time")?);
        let key_count = next(&mut stream, "key count")? as i32;
        if key_count < 0 {
            return Err(malformed(
                "StreamedClip",
                format!("negative key count {key_count}"),
            ));
        }
        let mut keys = Vec::with_capacity(key_count as usize);
        for _ in 0..key_count {
            let index = next(&mut stream, "curve index")?;
            let mut coeff = [0.0; 4];
            for c in &mut coeff {
                *c = f32::from_bits(next(&mut stream, "coefficient")?);
            }
            keys.push((index, coeff));
        }
        frames.push(StreamedFrame { time, keys });
    }
    Ok(frames)
}

fn field<'a>(value: &'a Value, path: &[&str]) -> Result<&'a Value> {
    path.iter().try_fold(value, |node, key| {
        node.get(key)
            .ok_or_else(|| malformed("AnimationClip", format!("missing field {}", path.join("."))))
    })
}

fn as_f32(value: &Value, what: &'static str) -> Result<f32> {
    value
        .as_f64()
        .map(|v| v as f32)
        .ok_or_else(|| malformed(what, format!("expected a number, got {value}")))
}

fn as_u64(value: &Value, what: &'static str) -> Result<u64> {
    value
        .as_u64()
        .ok_or_else(|| malformed(what, format!("expected an unsigned integer, got {value}")))
}

fn f32_array(value: &Value, what: &'static str) -> Result<Vec<f32>> {
    value
        .as_array()
        .ok_or_else(|| malformed(what, "expected an array"))?
        .iter()
        .map(|v| as_f32(v, what))
        .collect()
}

/// Converts one clip. `meta` is the `Live2DBuildMotionMetaData` typetree of the same name, if any.
/// The curves a `Transform` binding stands for (`typeID` 4, attribute 1–4), in Unity's order.
fn transform_components(type_id: i64, attribute: u32) -> Option<&'static [&'static str]> {
    if type_id != 4 {
        return None;
    }
    Some(match attribute {
        1 => &[
            "m_LocalPosition.x",
            "m_LocalPosition.y",
            "m_LocalPosition.z",
        ],
        2 => &[
            "m_LocalRotation.x",
            "m_LocalRotation.y",
            "m_LocalRotation.z",
            "m_LocalRotation.w",
        ],
        3 => &["m_LocalScale.x", "m_LocalScale.y", "m_LocalScale.z"],
        4 => &[
            "localEulerAnglesRaw.x",
            "localEulerAnglesRaw.y",
            "localEulerAnglesRaw.z",
        ],
        _ => return None,
    })
}

pub fn clip_to_motion(
    clip: &Value,
    meta: Option<&Value>,
    names: &BindingNames,
    source: Source,
) -> Result<SseMotion> {
    if clip.get("m_Compressed").and_then(Value::as_bool) == Some(true) {
        return Err(ConvertError::Unsupported(
            "compressed AnimationClip",
            source.bundle,
        ));
    }
    let muscle = field(clip, &["m_MuscleClip"])?;
    let data = field(muscle, &["m_Clip", "data"])?;

    let streamed = field(data, &["m_StreamedClip"])?;
    let streamed_count =
        as_u64(field(streamed, &["curveCount"])?, "StreamedClip.curveCount")? as usize;
    if let Some(discrete) = streamed.get("discreteCurveCount").and_then(Value::as_u64)
        && discrete != 0
    {
        return Err(ConvertError::Unsupported(
            "discrete StreamedClip curves",
            format!("{discrete} in {}", source.bundle),
        ));
    }
    let words: Vec<u32> = field(streamed, &["data"])?
        .as_array()
        .ok_or_else(|| malformed("StreamedClip.data", "expected an array"))?
        .iter()
        .map(|w| as_u64(w, "StreamedClip.data").map(|w| w as u32))
        .collect::<Result<_>>()?;

    let mut segments = vec![Vec::new(); streamed_count];
    for frame in decode_streamed(&words)? {
        if frame.time == f32::INFINITY {
            // Unity terminates the stream with a key-less +inf frame.
            continue;
        }
        for (index, coeff) in frame.keys {
            let curve = segments.get_mut(index as usize).ok_or_else(|| {
                malformed(
                    "StreamedClip",
                    format!("curve index {index} ≥ {streamed_count}"),
                )
            })?;
            curve.push(Segment {
                t: frame.time,
                coeff,
            });
        }
    }

    let dense = field(data, &["m_DenseClip"])?;
    let dense_count = as_u64(field(dense, &["m_CurveCount"])?, "DenseClip.m_CurveCount")? as usize;
    let dense_samples = f32_array(field(dense, &["m_SampleArray"])?, "DenseClip.m_SampleArray")?;
    let dense_begin = as_f32(field(dense, &["m_BeginTime"])?, "DenseClip.m_BeginTime")?;
    let dense_rate = as_f32(field(dense, &["m_SampleRate"])?, "DenseClip.m_SampleRate")?;
    let constants = f32_array(
        field(data, &["m_ConstantClip", "data"])?,
        "ConstantClip.data",
    )?;

    let mut curve_data: Vec<CurveData> = segments
        .into_iter()
        .map(|segments| CurveData::Streamed { segments })
        .collect();
    curve_data.extend((0..dense_count).map(|curve| {
        CurveData::Dense {
            begin_time: dense_begin,
            sample_rate: dense_rate,
            samples: dense_samples
                .iter()
                .skip(curve)
                .step_by(dense_count.max(1))
                .copied()
                .collect(),
        }
    }));
    curve_data.extend(
        constants
            .into_iter()
            .map(|value| CurveData::Constant { value }),
    );

    let bindings = field(clip, &["m_ClipBindingConstant", "genericBindings"])?
        .as_array()
        .ok_or_else(|| malformed("genericBindings", "expected an array"))?;
    // one entry per curve: (binding, attribute hash, attribute name)
    let mut per_curve = Vec::with_capacity(curve_data.len());
    for binding in bindings {
        let attr_hash = as_u64(field(binding, &["attribute"])?, "binding.attribute")? as u32;
        let type_id = field(binding, &["typeID"])?.as_i64().unwrap_or_default();
        match transform_components(type_id, attr_hash) {
            Some(names) => {
                for name in names {
                    per_curve.push((binding, crc32fast::hash(name.as_bytes()), Some(*name)));
                }
            }
            None => per_curve.push((binding, attr_hash, attribute_name(attr_hash))),
        }
    }
    if per_curve.len() != curve_data.len() {
        return Err(malformed(
            "AnimationClip",
            format!(
                "{} bindings ({} curves) for {} curves in {}",
                bindings.len(),
                per_curve.len(),
                curve_data.len(),
                source.bundle
            ),
        ));
    }
    let curves = per_curve
        .into_iter()
        .zip(curve_data)
        .map(|((binding, attr_hash, attr), data)| {
            let path_hash = as_u64(field(binding, &["path"])?, "binding.path")? as u32;
            Ok(Curve {
                binding: Binding {
                    path_hash,
                    attr_hash,
                    type_id: field(binding, &["typeID"])?.as_i64().unwrap_or_default() as i32,
                    custom_type: field(binding, &["customType"])?
                        .as_u64()
                        .unwrap_or_default() as u8,
                    resolved: names.resolve(path_hash),
                    attr: attr.map(str::to_owned),
                },
                data,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    let events = field(clip, &["m_Events"])?
        .as_array()
        .ok_or_else(|| malformed("m_Events", "expected an array"))?
        .iter()
        .map(|event| {
            Ok(Event {
                time: as_f32(field(event, &["time"])?, "event.time")?,
                function: field(event, &["functionName"])?
                    .as_str()
                    .unwrap_or_default()
                    .to_owned(),
                data: field(event, &["data"])?
                    .as_str()
                    .unwrap_or_default()
                    .to_owned(),
                float_parameter: as_f32(
                    field(event, &["floatParameter"])?,
                    "event.floatParameter",
                )?,
                int_parameter: field(event, &["intParameter"])?
                    .as_i64()
                    .unwrap_or_default() as i32,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    let fade = meta
        .map(|meta| {
            Ok::<_, ConvertError>(Fade {
                fade_in: as_f32(field(meta, &["FadeInTime"])?, "FadeInTime")?,
                fade_out: as_f32(field(meta, &["FadeOutTime"])?, "FadeOutTime")?,
            })
        })
        .transpose()?;

    Ok(SseMotion {
        format: motion::FORMAT.to_owned(),
        version: motion::VERSION,
        name: field(clip, &["m_Name"])?
            .as_str()
            .unwrap_or_default()
            .to_owned(),
        sample_rate: as_f32(field(clip, &["m_SampleRate"])?, "m_SampleRate")?,
        start_time: as_f32(field(muscle, &["m_StartTime"])?, "m_StartTime")?,
        stop_time: as_f32(field(muscle, &["m_StopTime"])?, "m_StopTime")?,
        loop_time: field(muscle, &["m_LoopTime"])?
            .as_bool()
            .unwrap_or_default(),
        fade,
        curves,
        events,
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    type Frame<'a> = (f32, &'a [(u32, [f32; 4])]);

    fn words(parts: &[Frame<'_>]) -> Vec<u32> {
        let mut out = Vec::new();
        for (time, keys) in parts {
            out.push(time.to_bits());
            out.push(keys.len() as u32);
            for (index, coeff) in *keys {
                out.push(*index);
                out.extend(coeff.iter().map(|c| c.to_bits()));
            }
        }
        out
    }

    fn source() -> Source {
        Source {
            bundle: "live2d/motion/test".into(),
            path_id: 1,
            container: None,
        }
    }

    #[test]
    fn decodes_frames_including_both_sentinels() {
        let stream = words(&[
            (-f32::MAX, &[(0, [0.0, 0.0, 0.0, 1.0])]),
            (0.5, &[(0, [1.0, 2.0, 3.0, 4.0])]),
            (f32::INFINITY, &[]),
        ]);
        let frames = decode_streamed(&stream).unwrap();
        assert_eq!(frames.len(), 3);
        assert_eq!(frames[1].keys, [(0, [1.0, 2.0, 3.0, 4.0])]);
        assert!(decode_streamed(&stream[..stream.len() - 1]).is_err());
    }

    #[test]
    fn converts_a_minimal_clip_and_resolves_bindings() {
        let path = crc32fast::hash(b"Parameters/ParamAngleX");
        let stream = words(&[
            (-f32::MAX, &[(0, [0.0, 0.0, 0.0, 0.0])]),
            (0.0, &[(0, [-24.635_777, 16.000_938, 0.0, 0.0])]),
            (f32::INFINITY, &[]),
        ]);
        let clip = json!({
            "m_Name": "w-test01", "m_Compressed": false, "m_SampleRate": 60.0,
            "m_MuscleClip": {
                "m_StartTime": 0.0, "m_StopTime": 2.45, "m_LoopTime": false,
                "m_Clip": {"data": {
                    "m_StreamedClip": {"data": stream, "curveCount": 1, "discreteCurveCount": 0},
                    "m_DenseClip": {"m_FrameCount": 0, "m_CurveCount": 0, "m_SampleRate": 60.0, "m_BeginTime": 0.0, "m_SampleArray": []},
                    "m_ConstantClip": {"data": [0.25]}
                }}
            },
            "m_ClipBindingConstant": {"genericBindings": [
                {"path": path, "attribute": motion::ATTR_VALUE, "typeID": 114, "customType": 0},
                {"path": 12345, "attribute": crc32fast::hash(b"EyeOpening"), "typeID": 114, "customType": 0}
            ]},
            "m_Events": [{"time": 0.183, "functionName": "OnLive2DInvokeUserData", "data": "eyeblink,0.2,0.79,0.24", "floatParameter": 0.0, "intParameter": 0}]
        });
        let meta = json!({"FadeInTime": 0.5, "FadeOutTime": 0.5});
        let mut names = BindingNames::default();
        names.add_model(&["ParamAngleX".into()], &[]);

        let motion = clip_to_motion(&clip, Some(&meta), &names, source()).unwrap();
        assert_eq!(motion.curves.len(), 2);
        assert_eq!(
            motion.curves[0].binding.resolved.as_ref().unwrap().id,
            "ParamAngleX"
        );
        assert_eq!(motion.curves[0].binding.attr.as_deref(), Some("Value"));
        assert_eq!(motion.curves[1].binding.resolved, None);
        assert_eq!(motion.curves[1].binding.attr.as_deref(), Some("EyeOpening"));
        let CurveData::Streamed { segments } = &motion.curves[0].data else {
            panic!()
        };
        assert_eq!(segments.len(), 2);
        assert_eq!(segments[1].coeff[0].to_bits(), (-24.635_777_f32).to_bits());
        assert_eq!(motion.curves[1].data, CurveData::Constant { value: 0.25 });
        assert_eq!(
            motion.fade,
            Some(Fade {
                fade_in: 0.5,
                fade_out: 0.5
            })
        );
        assert_eq!(motion.events[0].data, "eyeblink,0.2,0.79,0.24");
    }

    #[test]
    fn rejects_binding_count_mismatch() {
        let clip = json!({
            "m_Name": "x", "m_SampleRate": 60.0,
            "m_MuscleClip": {"m_StartTime": 0.0, "m_StopTime": 1.0, "m_LoopTime": false, "m_Clip": {"data": {
                "m_StreamedClip": {"data": [], "curveCount": 0},
                "m_DenseClip": {"m_CurveCount": 0, "m_SampleRate": 60.0, "m_BeginTime": 0.0, "m_SampleArray": []},
                "m_ConstantClip": {"data": [1.0]}
            }}},
            "m_ClipBindingConstant": {"genericBindings": []},
            "m_Events": []
        });
        assert!(clip_to_motion(&clip, None, &BindingNames::default(), source()).is_err());
    }

    #[test]
    fn transform_bindings_expand_to_their_component_curves() {
        let names = transform_components(4, 1).unwrap();
        assert_eq!(
            names,
            [
                "m_LocalPosition.x",
                "m_LocalPosition.y",
                "m_LocalPosition.z"
            ]
        );
        assert_eq!(transform_components(4, 2).unwrap().len(), 4);
        assert!(transform_components(224, 1).is_none());
        assert!(transform_components(4, 9).is_none());
    }
}
