use crate::manifest::Attenuation;

/// The listener pose: position and the world-space direction of the listener's right ear.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Listener {
    pub position: [f32; 3],
    pub right: [f32; 3],
}

impl Default for Listener {
    fn default() -> Self {
        Self {
            position: [0.0; 3],
            right: [1.0, 0.0, 0.0],
        }
    }
}

/// Linear distance fade: 1 up to `min_distance`, 0 from `max_distance`.
pub fn distance_gain(distance: f32, attenuation: &Attenuation) -> f32 {
    let Attenuation {
        min_distance,
        max_distance,
    } = *attenuation;
    if distance <= min_distance {
        1.0
    } else if distance >= max_distance {
        0.0
    } else {
        1.0 - (distance - min_distance) / (max_distance - min_distance)
    }
}

/// Stereo pan in `-1.0..=1.0` (left..right): the cosine between the listener's right axis
/// and the direction to the source. Inside `min_distance` the pan narrows linearly towards
/// the centre so a source passing through the listener does not flip sides abruptly.
///
/// Backends apply it with an equal-power law (kira `Panning`, Web Audio `StereoPannerNode`).
pub fn stereo_pan(listener: &Listener, source: [f32; 3], attenuation: &Attenuation) -> f32 {
    let offset = sub(source, listener.position);
    let distance = length(offset);
    let right_len = length(listener.right);
    if distance <= f32::EPSILON || right_len <= f32::EPSILON {
        return 0.0;
    }
    let cosine = dot(offset, listener.right) / (distance * right_len);
    let near = if attenuation.min_distance > 0.0 {
        (distance / attenuation.min_distance).min(1.0)
    } else {
        1.0
    };
    (cosine * near).clamp(-1.0, 1.0)
}

/// `(gain, pan)` for a source at `source`.
pub fn spatialize(listener: &Listener, source: [f32; 3], attenuation: &Attenuation) -> (f32, f32) {
    let distance = length(sub(source, listener.position));
    (
        distance_gain(distance, attenuation),
        stereo_pan(listener, source, attenuation),
    )
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn length(a: [f32; 3]) -> f32 {
    dot(a, a).sqrt()
}
