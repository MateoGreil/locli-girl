use num_complex::Complex;
use rustfft::FftPlanner;

pub fn compute_bars(samples: &[f32], n_bars: usize, _sample_rate: f32) -> Vec<f32> {
    if samples.is_empty() || n_bars == 0 {
        return vec![0.0; n_bars];
    }
    let fft_size = samples.len().next_power_of_two();
    let mut buffer: Vec<Complex<f32>> = (0..fft_size)
        .map(|i| {
            let s = samples.get(i).copied().unwrap_or(0.0);
            let w = 0.5
                * (1.0
                    - (2.0 * std::f32::consts::PI * i as f32 / (fft_size - 1) as f32).cos());
            Complex::new(s * w, 0.0)
        })
        .collect();

    let mut planner = FftPlanner::<f32>::new();
    planner.plan_fft_forward(fft_size).process(&mut buffer);

    let half = fft_size / 2;
    let magnitudes: Vec<f32> = buffer[..half].iter().map(|c| c.norm()).collect();

    let bars: Vec<f32> = (0..n_bars)
        .map(|bar| {
            let lo = (bar * half / n_bars).min(half - 1);
            let hi = ((bar + 1) * half / n_bars).max(lo + 1).min(half);
            magnitudes[lo..hi].iter().cloned().fold(0.0f32, f32::max)
        })
        .collect();

    let max = bars.iter().cloned().fold(0.0f32, f32::max);
    if max > 0.0 {
        bars.into_iter().map(|b| (b / max).min(1.0)).collect()
    } else {
        vec![0.0; n_bars]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    #[test]
    fn silence_produces_zero_bars() {
        let bars = compute_bars(&vec![0.0f32; 1024], 16, 44100.0);
        assert!(bars.iter().all(|&b| b == 0.0));
    }

    #[test]
    fn output_length_matches_n_bars() {
        let bars = compute_bars(&vec![0.5f32; 1024], 24, 44100.0);
        assert_eq!(bars.len(), 24);
    }

    #[test]
    fn empty_input_returns_zero_bars() {
        let bars = compute_bars(&[], 8, 44100.0);
        assert_eq!(bars, vec![0.0f32; 8]);
    }

    #[test]
    fn bars_normalized_between_0_and_1() {
        let sample_rate = 44100.0f32;
        let samples: Vec<f32> = (0..2048)
            .map(|i| (2.0 * PI * 440.0 * i as f32 / sample_rate).sin())
            .collect();
        let bars = compute_bars(&samples, 32, sample_rate);
        assert!(bars.iter().all(|&b| (0.0..=1.0).contains(&b)));
    }

    #[test]
    fn sine_440hz_peaks_in_low_frequency_bars() {
        let sample_rate = 44100.0f32;
        let samples: Vec<f32> = (0..4096)
            .map(|i| (2.0 * PI * 440.0 * i as f32 / sample_rate).sin())
            .collect();
        let bars = compute_bars(&samples, 32, sample_rate);
        let peak = bars
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .unwrap()
            .0;
        assert!(peak < 8, "440 Hz should peak in first quarter of bars, got bar {}", peak);
    }
}
