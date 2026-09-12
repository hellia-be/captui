//! See docs/DESIGN-NOTES.md.

pub fn samples_peak(bytes: &[u8]) -> f32 {
    let mut peak = 0.0f32;
    let mut i = 0;
    while i + 4 <= bytes.len() {
        let v = f32::from_le_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]).abs();
        if v > peak {
            peak = v;
        }
        i += 4;
    }
    peak.min(1.0)
}

pub fn meter_bar(level: f32, width: usize) -> String {
    let filled = (level.clamp(0.0, 1.0) * width as f32).round() as usize;
    let filled = filled.min(width);
    let mut s = String::with_capacity(width * 3);
    for i in 0..width {
        s.push(if i < filled { '█' } else { '░' });
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peak_of_f32_samples() {
        let mut bytes = Vec::new();
        for v in [0.1f32, -0.7, 0.3] {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        assert!((samples_peak(&bytes) - 0.7).abs() < 1e-6);
    }

    #[test]
    fn peak_is_clamped_and_empty_is_zero() {
        assert_eq!(samples_peak(&[]), 0.0);
        assert_eq!(samples_peak(&2.0f32.to_le_bytes()), 1.0);
    }

    #[test]
    fn bar_fills_proportionally() {
        assert_eq!(meter_bar(0.0, 10), "░░░░░░░░░░");
        assert_eq!(meter_bar(1.0, 10), "██████████");
        assert_eq!(meter_bar(0.5, 10), "█████░░░░░");
    }
}
