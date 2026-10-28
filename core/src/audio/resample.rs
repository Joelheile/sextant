//! Downmix and resample to 16 kHz mono (plan.md §4). A streaming windowed-sinc resampler: the
//! low-pass sits at the new Nyquist, so 48 kHz audio does not fold high tones into the band the
//! model listens to.

use super::RATE;

/// Zero crossings of the sinc on each side of the centre, at the lower of the two rates.
const ZEROS: f64 = 12.0;
/// Kernel table steps per input sample. Lookups interpolate linearly between them.
const STEPS: usize = 64;

struct Resampler {
    /// Input samples per output sample.
    ratio: f64,
    /// Input samples on each side of an output sample that the kernel reaches.
    half: usize,
    kernel: Vec<f32>,
    /// Input not yet consumed, starting `half` samples before the next output position.
    buf: Vec<f32>,
    /// Position of the next output sample in `buf`.
    at: f64,
}

impl Resampler {
    fn new(in_rate: u32) -> Self {
        let ratio = f64::from(in_rate) / f64::from(RATE);
        // Cutoff as a fraction of the input Nyquist: lower when the rate goes down.
        let cutoff = (1.0 / ratio).min(1.0);
        let half = (ZEROS / cutoff).ceil() as usize;
        let kernel = (0..=half * STEPS)
            .map(|step| {
                let d = step as f64 / STEPS as f64;
                let x = std::f64::consts::PI * d * cutoff;
                let sinc = if x == 0.0 { 1.0 } else { x.sin() / x };
                let window = 0.5 + 0.5 * (std::f64::consts::PI * d / half as f64).cos();
                (sinc * window * cutoff) as f32
            })
            .collect();
        Self {
            ratio,
            half,
            kernel,
            buf: vec![0.0; half],
            at: half as f64,
        }
    }

    fn weight(&self, distance: f64) -> f32 {
        let scaled = distance.abs() * STEPS as f64;
        let index = scaled as usize;
        match (self.kernel.get(index), self.kernel.get(index + 1)) {
            (Some(a), Some(b)) => a + (b - a) * (scaled - index as f64) as f32,
            _ => 0.0,
        }
    }

    fn push(&mut self, input: &[f32], out: &mut Vec<f32>) {
        self.buf.extend_from_slice(input);
        while self.at + self.half as f64 + 1.0 < self.buf.len() as f64 {
            let centre = self.at.floor() as usize;
            let mut sum = 0.0;
            for i in centre + 1 - self.half..=centre + self.half {
                sum += self.buf[i] * self.weight(i as f64 - self.at);
            }
            out.push(sum);
            self.at += self.ratio;
        }
        let drop = (self.at.floor() as usize).saturating_sub(self.half);
        self.buf.drain(..drop);
        self.at -= drop as f64;
    }
}

/// Interleaved samples at any rate in, 16 kHz mono out.
pub struct To16k {
    rate: u32,
    resampler: Option<Resampler>,
    mono: Vec<f32>,
}

impl Default for To16k {
    fn default() -> Self {
        Self::new()
    }
}

impl To16k {
    pub fn new() -> Self {
        Self {
            rate: RATE,
            resampler: None,
            mono: Vec::new(),
        }
    }

    /// Appends the converted samples to `out`. A change of `rate` starts a fresh filter.
    pub fn push(&mut self, interleaved: &[f32], channels: u16, rate: u32, out: &mut Vec<f32>) {
        let channels = usize::from(channels.max(1));
        if rate != self.rate || (rate != RATE && self.resampler.is_none()) {
            self.rate = rate;
            self.resampler = (rate != RATE && rate > 0).then(|| Resampler::new(rate));
        }
        self.mono.clear();
        self.mono.extend(
            interleaved
                .chunks_exact(channels)
                .map(|frame| frame.iter().sum::<f32>() / channels as f32),
        );
        match &mut self.resampler {
            Some(resampler) => resampler.push(&self.mono, out),
            None => out.extend_from_slice(&self.mono),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(hz: f32, rate: u32, seconds: f32) -> Vec<f32> {
        (0..(rate as f32 * seconds) as usize)
            .map(|n| (2.0 * std::f32::consts::PI * hz * n as f32 / rate as f32).sin())
            .collect()
    }

    fn rms(samples: &[f32]) -> f32 {
        (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
    }

    fn convert(input: &[f32], rate: u32, block: usize) -> Vec<f32> {
        let mut converter = To16k::new();
        let mut out = Vec::new();
        for chunk in input.chunks(block) {
            converter.push(chunk, 1, rate, &mut out);
        }
        out
    }

    #[test]
    fn a_tone_keeps_its_level_and_length_from_48_khz() {
        let out = convert(&tone(1000.0, 48_000, 1.0), 48_000, 480);
        assert!((out.len() as i64 - 16_000).abs() < 40, "{}", out.len());
        let level = rms(&out[1000..out.len() - 1000]);
        assert!(
            (level - std::f32::consts::FRAC_1_SQRT_2).abs() < 0.02,
            "{level}"
        );
    }

    #[test]
    fn blocks_of_any_size_give_the_same_samples() {
        let input = tone(440.0, 44_100, 0.5);
        let whole = convert(&input, 44_100, input.len());
        let small = convert(&input, 44_100, 97);
        assert_eq!(whole.len(), small.len());
        for (a, b) in whole.iter().zip(&small) {
            assert!((a - b).abs() < 1e-4);
        }
    }

    #[test]
    fn tones_above_the_new_nyquist_are_filtered_out() {
        let out = convert(&tone(12_000.0, 48_000, 1.0), 48_000, 480);
        assert!(rms(&out[1000..out.len() - 1000]) < 0.02);
    }

    #[test]
    fn stereo_is_averaged_and_16_khz_passes_through() {
        let mut converter = To16k::new();
        let mut out = Vec::new();
        converter.push(&[1.0, 0.0, 0.5, 0.5, -1.0, 1.0], 2, 16_000, &mut out);
        assert_eq!(out, [0.5, 0.5, 0.0]);
    }

    #[test]
    fn a_rate_change_starts_a_fresh_filter() {
        let mut converter = To16k::new();
        let mut out = Vec::new();
        converter.push(&tone(1000.0, 48_000, 0.2), 1, 48_000, &mut out);
        let before = out.len();
        converter.push(&tone(1000.0, 16_000, 0.1), 1, 16_000, &mut out);
        assert_eq!(out.len() - before, 1600);
    }
}
