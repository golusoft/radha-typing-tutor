//! Generate the UI sound samples in `assets/sounds/`.
//!
//! ```text
//! cargo run -p radha-app --example generate_sounds
//! ```
//!
//! Every sample is synthesised from pure math (sines, linear attack/release,
//! optional exponential decay) — no recordings, no randomness, no third-party
//! material — so the output is byte-for-byte deterministic and carries no
//! licence obligations. The generated `.wav` files are checked in; re-run this
//! only when a sound design changes.
//!
//! Format: RIFF/WAVE, PCM 16-bit, mono, 22,050 Hz. Peak amplitude 0.35 of full
//! scale keeps the cues soft. Playback is not implemented here (P06).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Output sample rate (Hz).
const SAMPLE_RATE: u32 = 22_050;
/// Peak amplitude as a fraction of full scale.
const PEAK: f64 = 0.35;
/// Linear fade-in and fade-out applied to every tone, in milliseconds.
const EDGE_MS: f64 = 3.0;

/// One synthesised tone segment.
struct Tone {
    /// Sine frequencies mixed with equal weight.
    freqs: &'static [f64],
    /// Segment length in milliseconds.
    duration_ms: f64,
    /// Exponential decay time constant in milliseconds; `None` = sustained.
    decay_ms: Option<f64>,
}

/// A sound file: one or more tones played back to back.
struct Sound {
    file: &'static str,
    tones: &'static [Tone],
}

const SOUNDS: &[Sound] = &[
    Sound {
        file: "key.wav",
        tones: &[Tone {
            freqs: &[1_800.0],
            duration_ms: 30.0,
            decay_ms: Some(6.0),
        }],
    },
    Sound {
        file: "error.wav",
        // Two close sines give a soft beat rather than a harsh buzz.
        tones: &[Tone {
            freqs: &[220.0, 233.0],
            duration_ms: 120.0,
            decay_ms: Some(50.0),
        }],
    },
    Sound {
        file: "success.wav",
        tones: &[
            Tone {
                freqs: &[660.0],
                duration_ms: 80.0,
                decay_ms: None,
            },
            Tone {
                freqs: &[880.0],
                duration_ms: 100.0,
                decay_ms: None,
            },
        ],
    },
    Sound {
        file: "complete.wav",
        // C5 – E5 – G5 major arpeggio.
        tones: &[
            Tone {
                freqs: &[523.25],
                duration_ms: 150.0,
                decay_ms: None,
            },
            Tone {
                freqs: &[659.25],
                duration_ms: 150.0,
                decay_ms: None,
            },
            Tone {
                freqs: &[783.99],
                duration_ms: 150.0,
                decay_ms: None,
            },
        ],
    },
    Sound {
        file: "timer.wav",
        tones: &[Tone {
            freqs: &[1_000.0],
            duration_ms: 80.0,
            decay_ms: Some(20.0),
        }],
    },
];

/// Number of samples covering `ms` milliseconds.
fn samples_for(ms: f64) -> usize {
    // Durations are small positive constants, so the cast cannot truncate.
    (ms * f64::from(SAMPLE_RATE) / 1_000.0).round() as usize
}

/// Render one tone into 16-bit samples.
fn render(tone: &Tone, out: &mut Vec<i16>) {
    let n = samples_for(tone.duration_ms);
    let edge = samples_for(EDGE_MS).max(1);
    let rate = f64::from(SAMPLE_RATE);
    let voices = tone.freqs.len().max(1) as f64;

    for i in 0..n {
        let t = i as f64 / rate;
        let mix: f64 = tone
            .freqs
            .iter()
            .map(|f| (2.0 * std::f64::consts::PI * f * t).sin())
            .sum::<f64>()
            / voices;

        // Linear attack and release remove clicks at segment boundaries.
        let attack = ((i + 1) as f64 / edge as f64).min(1.0);
        let release = ((n - i) as f64 / edge as f64).min(1.0);
        let decay = tone
            .decay_ms
            .map_or(1.0, |tau| (-(t * 1_000.0) / tau).exp());

        let value = mix * PEAK * attack * release * decay;
        // |value| <= PEAK < 1, so the scaled result always fits in i16.
        out.push((value * f64::from(i16::MAX)).round() as i16);
    }
}

/// Encode samples as a canonical 44-byte-header PCM WAV.
fn encode_wav(samples: &[i16]) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let channels: u16 = 1;
    let bits: u16 = 16;
    let block_align = channels * bits / 8;
    let byte_rate = SAMPLE_RATE * u32::from(block_align);

    let mut buf = Vec::with_capacity(44 + samples.len() * 2);
    buf.extend_from_slice(b"RIFF");
    buf.extend_from_slice(&(36 + data_len).to_le_bytes());
    buf.extend_from_slice(b"WAVE");
    buf.extend_from_slice(b"fmt ");
    buf.extend_from_slice(&16u32.to_le_bytes());
    buf.extend_from_slice(&1u16.to_le_bytes()); // PCM
    buf.extend_from_slice(&channels.to_le_bytes());
    buf.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    buf.extend_from_slice(&byte_rate.to_le_bytes());
    buf.extend_from_slice(&block_align.to_le_bytes());
    buf.extend_from_slice(&bits.to_le_bytes());
    buf.extend_from_slice(b"data");
    buf.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        buf.extend_from_slice(&s.to_le_bytes());
    }
    buf
}

fn output_dir() -> PathBuf {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/sounds")).to_path_buf()
}

fn main() -> io::Result<()> {
    let dir = output_dir();
    fs::create_dir_all(&dir)?;

    let mut total = 0usize;
    for sound in SOUNDS {
        let mut samples = Vec::new();
        for tone in sound.tones {
            render(tone, &mut samples);
        }
        let bytes = encode_wav(&samples);
        let path = dir.join(sound.file);
        fs::write(&path, &bytes)?;
        total += bytes.len();
        println!(
            "{} — {} samples, {} bytes",
            sound.file,
            samples.len(),
            bytes.len()
        );
    }
    println!("total {total} bytes in {}", dir.display());
    Ok(())
}
