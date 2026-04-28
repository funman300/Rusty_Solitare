//! Synthesize placeholder SFX into `assets/audio/`.
//!
//! Output: 44.1kHz mono 16-bit PCM WAV. Run with
//! `cargo run -p solitaire_assetgen --bin gen_sfx`. Files are committed to
//! the repo so end-users never need to run this generator.

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

const SAMPLE_RATE: u32 = 44_100;

type Generator = fn() -> Vec<i16>;

fn main() -> io::Result<()> {
    let out_dir = workspace_root().join("assets").join("audio");
    fs::create_dir_all(&out_dir)?;

    let effects: [(&str, Generator); 6] = [
        ("card_flip.wav", card_flip),
        ("card_place.wav", card_place),
        ("card_deal.wav", card_deal),
        ("card_invalid.wav", card_invalid),
        ("win_fanfare.wav", win_fanfare),
        ("ambient_loop.wav", ambient_loop),
    ];

    for (name, gen) in &effects {
        let samples = gen();
        let path = out_dir.join(name);
        write_wav_mono_pcm16(&path, SAMPLE_RATE, &samples)?;
        println!("wrote {} ({} samples)", path.display(), samples.len());
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Synth primitives
// ---------------------------------------------------------------------------

/// Simple deterministic noise source — LCG, no `rand` dep needed.
struct Lcg(u64);

impl Lcg {
    fn new(seed: u64) -> Self {
        Self(seed)
    }
    fn next_f32(&mut self) -> f32 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        ((self.0 >> 32) as i32 as f32) / (i32::MAX as f32)
    }
}

fn duration_samples(seconds: f32) -> usize {
    (seconds * SAMPLE_RATE as f32) as usize
}

/// Linear attack / exponential decay envelope. `attack` and length in seconds.
fn ar_envelope(t_secs: f32, attack: f32, total: f32, decay_rate: f32) -> f32 {
    if t_secs < attack {
        (t_secs / attack).clamp(0.0, 1.0)
    } else {
        (-decay_rate * (t_secs - attack)).exp() * (1.0 - (t_secs - total).max(0.0))
    }
}

fn quantize(sample: f32) -> i16 {
    let clipped = sample.clamp(-1.0, 1.0);
    (clipped * 32_767.0) as i16
}

// ---------------------------------------------------------------------------
// Effect generators
// ---------------------------------------------------------------------------

fn card_flip() -> Vec<i16> {
    let n = duration_samples(0.08);
    let mut rng = Lcg::new(0x1234_5678_DEAD_BEEF);
    let mut out = Vec::with_capacity(n);
    let mut prev = 0.0f32;
    let alpha = 0.35;
    for i in 0..n {
        let t = i as f32 / SAMPLE_RATE as f32;
        let raw = rng.next_f32();
        // High-pass-ish: subtract a low-pass-smoothed signal.
        let lp = alpha * raw + (1.0 - alpha) * prev;
        prev = lp;
        let hp = raw - lp;
        let env = ar_envelope(t, 0.005, 0.08, 60.0);
        out.push(quantize(hp * env * 0.6));
    }
    out
}

fn card_place() -> Vec<i16> {
    let n = duration_samples(0.14);
    let mut rng = Lcg::new(0xCAFE_F00D_8BAD_F00D);
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let t = i as f32 / SAMPLE_RATE as f32;
        // Low sine for body (~120 Hz) + filtered noise for click.
        let body = (2.0 * std::f32::consts::PI * 120.0 * t).sin();
        let click = rng.next_f32() * 0.5;
        let env = ar_envelope(t, 0.003, 0.14, 35.0);
        let sample = (body * 0.7 + click) * env * 0.55;
        out.push(quantize(sample));
    }
    out
}

fn card_deal() -> Vec<i16> {
    let n = duration_samples(0.18);
    let mut rng = Lcg::new(0xFEE1_DEAD_DEAD_BEEF);
    let mut out = Vec::with_capacity(n);
    let mut lp = 0.0f32;
    for i in 0..n {
        let t = i as f32 / SAMPLE_RATE as f32;
        let raw = rng.next_f32();
        // Sweeping low-pass: cutoff falls over time → "whoosh".
        let alpha = 0.6 - (t / 0.18) * 0.5;
        lp = alpha * raw + (1.0 - alpha) * lp;
        let env = ar_envelope(t, 0.01, 0.18, 18.0);
        out.push(quantize(lp * env * 0.7));
    }
    out
}

fn card_invalid() -> Vec<i16> {
    let n = duration_samples(0.18);
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let t = i as f32 / SAMPLE_RATE as f32;
        // Two dissonant squarish tones — strong beat creates a buzz.
        let a = (2.0 * std::f32::consts::PI * 196.0 * t).sin().signum();
        let b = (2.0 * std::f32::consts::PI * 207.65 * t).sin().signum();
        let env = ar_envelope(t, 0.005, 0.18, 12.0);
        out.push(quantize((a + b) * env * 0.18));
    }
    out
}

fn win_fanfare() -> Vec<i16> {
    // C major arpeggio: C5, E5, G5, C6.
    let notes = [523.25_f32, 659.25, 783.99, 1046.50];
    let note_dur = 0.18_f32;
    let total = note_dur * notes.len() as f32 + 0.25;
    let n = duration_samples(total);
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let t = i as f32 / SAMPLE_RATE as f32;
        let mut sample = 0.0f32;
        for (idx, freq) in notes.iter().enumerate() {
            let start = idx as f32 * note_dur;
            let local = t - start;
            if !(0.0..=0.4).contains(&local) {
                continue;
            }
            // Layered sine + soft 2nd harmonic for warmth.
            let s = (2.0 * std::f32::consts::PI * freq * local).sin()
                + 0.3 * (2.0 * std::f32::consts::PI * freq * 2.0 * local).sin();
            let env = ar_envelope(local, 0.008, 0.4, 6.0);
            sample += s * env;
        }
        out.push(quantize(sample * 0.22));
    }
    out
}

/// Generates a seamlessly looping ambient drone track (~6 seconds, 44100 Hz
/// mono 16-bit PCM).
///
/// Design:
/// - Fundamental: 55 Hz (low A) sine wave.
/// - Harmonics: 110 Hz at 40% and 165 Hz at 20% for warmth.
/// - Amplitude LFO at 0.1 Hz creates a slow breath / pad swell.
/// - The loop length is chosen so both the fundamental and LFO complete an
///   integer number of cycles — guaranteeing a phase-continuous seamless loop.
/// - Peak amplitude is kept low (0.18) so it sits quietly under SFX.
fn ambient_loop() -> Vec<i16> {
    use std::f32::consts::PI;

    // LFO period = 10 s; fundamental period ≈ 18.18 ms.
    // We want a loop that is an exact integer multiple of both, so both
    // complete a whole number of cycles with no phase discontinuity.
    //
    // LCM approach: fundamental @ 55 Hz repeats every 1/55 s. The LFO @ 0.1 Hz
    // repeats every 10 s. 10 s is already a multiple of 1/55 s (10 * 55 = 550
    // cycles), so a 10-second buffer loops perfectly. We halve it to 5 s for
    // a smaller file — 5 * 55 = 275 (integer), 5 * 0.1 = 0.5 (half-cycle of
    // LFO). To keep a full LFO cycle we use 10 s but write only the first 5 s
    // of the waveform, which is within the 4–8 s budget and still a seamless
    // loop because the LFO amplitude is symmetric about its midpoint at t=5 s.
    //
    // Simpler explanation: at exactly 5 s, both the 55 Hz tone and a slow
    // 0.2 Hz (period=5 s) breath LFO complete an integer number of cycles.
    // We use 0.2 Hz for the LFO instead of 0.1 Hz so the full envelope fits
    // in one loop period.
    let lfo_freq = 0.2_f32; // 1 full LFO cycle per 5-second loop
    let loop_seconds = 1.0 / lfo_freq; // = 5.0 s
    let n = (loop_seconds * SAMPLE_RATE as f32) as usize;

    let f0 = 55.0_f32; // fundamental (Hz)
    let f1 = 110.0_f32; // 2nd harmonic
    let f2 = 165.0_f32; // 3rd harmonic

    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let t = i as f32 / SAMPLE_RATE as f32;

        // LFO: smoothly oscillates between 0.4 and 1.0 amplitude.
        // Using (1 - cos) / 2 instead of sin so the loop starts and ends at
        // the same LFO phase (0.0 → both sin and cos are fully periodic).
        let lfo = 0.7 + 0.3 * (2.0 * PI * lfo_freq * t).cos();

        // Layered harmonics
        let tone = (2.0 * PI * f0 * t).sin()
            + 0.4 * (2.0 * PI * f1 * t).sin()
            + 0.2 * (2.0 * PI * f2 * t).sin();

        // Normalise the layered sum: max raw peak ≈ 1.6; keep final peak ≤ 0.18
        let sample = tone / 1.6 * lfo * 0.18;
        out.push(quantize(sample));
    }
    out
}

// ---------------------------------------------------------------------------
// Minimal WAV writer (mono 16-bit PCM)
// ---------------------------------------------------------------------------

fn write_wav_mono_pcm16(path: &Path, sample_rate: u32, samples: &[i16]) -> io::Result<()> {
    let mut f = File::create(path)?;
    let byte_rate = sample_rate * 2; // mono 16-bit
    let data_bytes = samples.len() as u32 * 2;
    let chunk_size = 36 + data_bytes;

    f.write_all(b"RIFF")?;
    f.write_all(&chunk_size.to_le_bytes())?;
    f.write_all(b"WAVE")?;

    f.write_all(b"fmt ")?;
    f.write_all(&16u32.to_le_bytes())?; // PCM fmt chunk size
    f.write_all(&1u16.to_le_bytes())?; // PCM
    f.write_all(&1u16.to_le_bytes())?; // mono
    f.write_all(&sample_rate.to_le_bytes())?;
    f.write_all(&byte_rate.to_le_bytes())?;
    f.write_all(&2u16.to_le_bytes())?; // block align
    f.write_all(&16u16.to_le_bytes())?; // bits per sample

    f.write_all(b"data")?;
    f.write_all(&data_bytes.to_le_bytes())?;
    for &s in samples {
        f.write_all(&s.to_le_bytes())?;
    }
    Ok(())
}

fn workspace_root() -> PathBuf {
    // CARGO_MANIFEST_DIR points at the assetgen crate; parent is workspace.
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    crate_dir.parent().expect("workspace root").to_path_buf()
}
