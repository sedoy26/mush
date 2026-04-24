//! DSP utilities with proper math and anti-aliasing.
//!
//! This module contains sample-rate-aware DSP functions that avoid
//! common pitfalls like discontinuities, denormals, and aliasing.

/// Tiny value to prevent denormals in feedback loops.
/// Adding this to feedback signals prevents CPU spikes on silence.
pub const DENORMAL_PREVENTION: f32 = 1e-24;

/// Maximum delay feedback to prevent infinite oscillation.
pub const MAX_FEEDBACK: f32 = 0.95;

/// Continuous soft limiter using tanh with drive normalization.
/// This is applied to ALL samples (not conditionally) to avoid discontinuities.
/// 
/// Formula: tanh(x * drive) / tanh(drive) * ceiling
/// 
/// - drive > 1.0 gives soft saturation
/// - The division by tanh(drive) normalizes so that small signals pass through ~unchanged
/// - ceiling sets the output maximum (GUARANTEED via final clamp)
#[inline]
pub fn soft_limit(sample: f32, drive: f32, ceiling: f32) -> f32 {
    let drive = drive.max(1.0);
    let result = (sample * drive).tanh() / drive.tanh() * ceiling;
    // CRITICAL: clamp to guarantee ceiling - the tanh math can exceed ceiling when drive > 1
    result.clamp(-ceiling, ceiling)
}

/// Calculate exponential decay coefficient for a given time constant.
/// This is sample-rate-aware and produces consistent decay across sample rates.
/// 
/// tau = time in seconds to decay to ~37% (1/e)
/// Returns the per-sample multiplier.
#[inline]
pub fn decay_coeff(tau: f32, sample_rate: f32) -> f32 {
    (-1.0 / (tau * sample_rate)).exp()
}

/// Calculate attack coefficient for exponential rise.
/// tau = time in seconds to reach ~63% of target
#[inline]
pub fn attack_coeff(tau: f32, sample_rate: f32) -> f32 {
    1.0 - (-1.0 / (tau * sample_rate)).exp()
}

/// PolyBLEP (Polynomial Bandlimited Step) anti-aliasing.
/// Reduces aliasing for discontinuous waveforms (saw, square).
/// 
/// t = current phase position where discontinuity occurs
/// dt = phase increment per sample (freq / sample_rate)
#[inline]
pub fn poly_blep(t: f32, dt: f32) -> f32 {
    if t < dt {
        // Just after discontinuity
        let t = t / dt;
        2.0 * t - t * t - 1.0
    } else if t > 1.0 - dt {
        // Just before discontinuity
        let t = (t - 1.0) / dt;
        t * t + 2.0 * t + 1.0
    } else {
        0.0
    }
}

/// Band-limited sawtooth oscillator using PolyBLEP.
#[inline]
pub fn saw_blep(phase: f32, dt: f32) -> f32 {
    let naive = 2.0 * phase - 1.0;
    naive - poly_blep(phase, dt)
}

/// Band-limited square oscillator using PolyBLEP.
#[inline]
pub fn square_blep(phase: f32, dt: f32) -> f32 {
    let naive = if phase < 0.5 { 1.0 } else { -1.0 };
    let mut out = naive;
    out += poly_blep(phase, dt);
    out -= poly_blep((phase + 0.5).fract(), dt);
    out
}

/// One-pole lowpass filter with denormal prevention.
#[inline]
pub fn one_pole_lp(input: f32, state: &mut f32, coeff: f32) -> f32 {
    *state = *state + (input - *state) * coeff + DENORMAL_PREVENTION;
    // Remove the DC offset we added
    *state - DENORMAL_PREVENTION
}

/// Simple allpass filter for reverb diffusion.
/// g = feedback coefficient (typically 0.5-0.7)
#[inline]
pub fn allpass(input: f32, buffer: &mut [f32], index: &mut usize, g: f32) -> f32 {
    let delayed = buffer[*index];
    let output = -input + delayed;
    buffer[*index] = input + delayed * g + DENORMAL_PREVENTION;
    *index = (*index + 1) % buffer.len();
    output
}

/// Comb filter with feedback and denormal prevention.
#[inline]
pub fn comb_filter(input: f32, buffer: &mut [f32], index: &mut usize, feedback: f32) -> f32 {
    let delayed = buffer[*index];
    // Add tiny noise to prevent denormals in feedback
    buffer[*index] = input + delayed * feedback.min(MAX_FEEDBACK) + DENORMAL_PREVENTION;
    *index = (*index + 1) % buffer.len();
    delayed
}

/// Smoothed envelope that always continues from current value.
/// Prevents clicks when retriggering during attack/release.
#[derive(Clone, Debug, Default)]
pub struct SmoothEnvelope {
    pub value: f32,
    attack_coeff: f32,
    release_coeff: f32,
}

impl SmoothEnvelope {
    pub fn new(attack_time: f32, release_time: f32, sample_rate: f32) -> Self {
        Self {
            value: 0.0,
            attack_coeff: attack_coeff(attack_time.max(0.001), sample_rate),
            release_coeff: decay_coeff(release_time.max(0.001), sample_rate),
        }
    }

    pub fn set_times(&mut self, attack_time: f32, release_time: f32, sample_rate: f32) {
        self.attack_coeff = attack_coeff(attack_time.max(0.001), sample_rate);
        self.release_coeff = decay_coeff(release_time.max(0.001), sample_rate);
    }

    /// Process one sample. Gate = true for note on, false for note off.
    /// Always continues from current value - never jumps to 0 or sustain.
    #[inline]
    pub fn process(&mut self, gate: bool) -> f32 {
        if gate {
            // Attack: rise toward 1.0 from current value
            self.value += (1.0 - self.value) * self.attack_coeff;
        } else {
            // Release: decay toward 0 from current value
            self.value *= self.release_coeff;
            // Snap to zero when very small to save CPU
            if self.value < 1e-6 {
                self.value = 0.0;
            }
        }
        self.value
    }
}

/// Simple Freeverb-style reverb with comb filters + allpass diffusers.
/// Much better sounding than comb-only "metallic" reverb.
pub struct SimpleReverb {
    comb_buffers: [Vec<f32>; 4],
    comb_indices: [usize; 4],
    allpass_buffers: [Vec<f32>; 2],
    allpass_indices: [usize; 2],
    feedback: f32,
    wet: f32,
}

impl SimpleReverb {
    /// Create a new reverb. decay = 0.0-1.0, wet = mix amount
    pub fn new(sample_rate: f32, decay: f32, wet: f32) -> Self {
        // Comb filter delays (in samples) - prime-ish numbers to reduce resonance
        let comb_times: [f32; 4] = [0.0297, 0.0371, 0.0411, 0.0437];
        let allpass_times: [f32; 2] = [0.005, 0.0017];

        Self {
            comb_buffers: comb_times.map(|t| vec![0.0; (t * sample_rate) as usize]),
            comb_indices: [0; 4],
            allpass_buffers: allpass_times.map(|t| vec![0.0; (t * sample_rate) as usize]),
            allpass_indices: [0; 2],
            feedback: 0.7 + decay * 0.28, // 0.7 to 0.98
            wet,
        }
    }

    pub fn set_params(&mut self, decay: f32, wet: f32) {
        self.feedback = (0.7 + decay * 0.28).min(MAX_FEEDBACK);
        self.wet = wet;
    }

    #[inline]
    pub fn process(&mut self, input: f32) -> f32 {
        if self.wet < 0.001 {
            return input;
        }

        // Sum of comb filters (parallel)
        let mut comb_sum = 0.0;
        for i in 0..4 {
            comb_sum += comb_filter(
                input,
                &mut self.comb_buffers[i],
                &mut self.comb_indices[i],
                self.feedback,
            );
        }
        comb_sum *= 0.25; // Normalize

        // Allpass diffusers (series)
        let mut diffused = comb_sum;
        for i in 0..2 {
            diffused = allpass(
                diffused,
                &mut self.allpass_buffers[i],
                &mut self.allpass_indices[i],
                0.5,
            );
        }

        // Mix dry and wet
        input * (1.0 - self.wet) + diffused * self.wet
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn soft_limit_is_continuous() {
        // Test that soft_limit produces no discontinuities around the knee
        let drive = 1.5;
        let ceiling = 0.92;
        
        // Sample near threshold
        let y1 = soft_limit(0.9, drive, ceiling);
        let y2 = soft_limit(0.91, drive, ceiling);
        let y3 = soft_limit(0.92, drive, ceiling);
        let y4 = soft_limit(0.93, drive, ceiling);
        
        // Should be monotonically increasing and smooth
        assert!(y2 > y1);
        assert!(y3 > y2);
        assert!(y4 > y3);
        
        // No large jumps (discontinuity check)
        assert!((y2 - y1).abs() < 0.1);
        assert!((y3 - y2).abs() < 0.1);
        assert!((y4 - y3).abs() < 0.1);
    }

    #[test]
    fn decay_coeff_is_rate_independent() {
        // Same tau should give same behavior at different sample rates
        let tau = 0.1; // 100ms
        
        let coeff_44k = decay_coeff(tau, 44100.0);
        let coeff_48k = decay_coeff(tau, 48000.0);
        
        // After tau seconds, should be at ~37% regardless of rate
        let samples_44k = (tau * 44100.0) as usize;
        let samples_48k = (tau * 48000.0) as usize;
        
        let mut val_44k = 1.0f32;
        for _ in 0..samples_44k {
            val_44k *= coeff_44k;
        }
        
        let mut val_48k = 1.0f32;
        for _ in 0..samples_48k {
            val_48k *= coeff_48k;
        }
        
        // Both should be near 1/e ≈ 0.368
        assert!((val_44k - 0.368).abs() < 0.02);
        assert!((val_48k - 0.368).abs() < 0.02);
    }

    #[test]
    fn envelope_continues_from_current() {
        let mut env = SmoothEnvelope::new(0.01, 0.1, 44100.0);
        
        // Attack partway
        for _ in 0..200 {
            env.process(true);
        }
        let mid_attack = env.value;
        assert!(mid_attack > 0.0 && mid_attack < 1.0);
        
        // Release should start from current value, not jump
        let before_release = env.value;
        env.process(false);
        let after_release = env.value;
        
        // Should decay smoothly, not jump to 0
        assert!(after_release < before_release);
        assert!(after_release > before_release * 0.5); // Not a huge jump
    }
}
