//! `sextant bench --asr` and `sextant bench --fetch-models`. The first runs
//! Parakeet over speech fixtures and prints the real-time factor (RTF: seconds of compute per
//! second of audio), the word error rate (WER) against the reference text, and memory with the
//! model loaded. The second puts the models in a dev folder through the code path the app uses.
//! Only numbers reach the output, never the transcribed text.
//!
//! Fixtures live in `<speech>/<language>/<id>.wav` (16 kHz mono, 16 bit) next to `<id>.txt`; the
//! `scripts/fetch-speech.sh` script makes them. Models default to `.dev-data/models`, which git
//! ignores.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use anyhow::{Context, Result, bail};

use super::usage;
use crate::audio::{RATE, Runs, Speech, Vad};
use crate::{models, platform};

const RTF_BUDGET: f64 = 0.2;
const RAM_BUDGET_MB: f64 = 1300.0;
const DEFAULT_MODELS: &str = ".dev-data/models";
const DEFAULT_SPEECH: &str = "fixtures/speech";
const VAD_FRAME: usize = 512;

pub fn fetch_models(dir: Option<&String>) -> Result<()> {
    let dir = dir.map_or_else(|| PathBuf::from(DEFAULT_MODELS), PathBuf::from);
    let report = |progress: models::Progress| {
        eprint!(
            "\r{:5.1} % of {:.0} MB",
            progress.done as f64 / progress.total as f64 * 100.0,
            progress.total as f64 / 1_048_576.0
        );
    };
    models::download(
        &dir,
        models::ASSETS,
        &models::UrlSession,
        &AtomicBool::new(false),
        &report,
    )?;
    eprintln!();
    println!("models are in {}", dir.display());
    Ok(())
}

struct Clip {
    samples: Vec<f32>,
    reference: String,
}

struct Language {
    name: String,
    clips: Vec<Clip>,
}

#[derive(Default)]
struct Totals {
    audio_s: f64,
    asr_s: f64,
    vad_s: f64,
    errors: usize,
    words: usize,
}

impl Totals {
    fn rtf(&self) -> f64 {
        self.asr_s / self.audio_s
    }

    fn wer(&self) -> f64 {
        self.errors as f64 / self.words.max(1) as f64 * 100.0
    }

    fn add(&mut self, other: &Totals) {
        self.audio_s += other.audio_s;
        self.asr_s += other.asr_s;
        self.vad_s += other.vad_s;
        self.errors += other.errors;
        self.words += other.words;
    }
}

pub fn asr(args: &[String]) -> Result<()> {
    let runs = if args.iter().any(|arg| arg == "--coreml") {
        Runs::CoreMl
    } else {
        Runs::Cpu
    };
    let paths: Vec<&String> = args.iter().filter(|arg| *arg != "--coreml").collect();
    let models_dir = paths
        .first()
        .map_or_else(|| PathBuf::from(DEFAULT_MODELS), PathBuf::from);
    let speech_dir = paths
        .get(1)
        .map_or_else(|| PathBuf::from(DEFAULT_SPEECH), PathBuf::from);
    if !models::installed(&models_dir, models::ASSETS) {
        bail!(
            "the models are not in {}: run `sextant bench --fetch-models` first",
            models_dir.display()
        );
    }
    let languages = load_fixtures(&speech_dir)?;
    if languages.is_empty() {
        bail!(
            "no speech fixtures in {}: run scripts/fetch-speech.sh",
            speech_dir.display()
        );
    }

    let mut speech = platform::Parakeet::new(&models_dir, runs);
    let (_, rss_before) = usage();
    let started = Instant::now();
    speech.load()?;
    let (_, rss_loaded) = usage();
    // The first run compiles and warms the model, so it is not part of the numbers.
    speech.transcribe(&languages[0].clips[0].samples)?;
    println!(
        "{runs:?}: model loaded and warmed up in {:.1} s",
        started.elapsed().as_secs_f64()
    );
    let mut vad = platform::silero(&models_dir)()?;

    let mut all = Totals::default();
    for language in &languages {
        let mut totals = Totals::default();
        for clip in &language.clips {
            let audio_s = clip.samples.len() as f64 / f64::from(RATE);
            let began = Instant::now();
            let text = speech.transcribe(&clip.samples)?;
            totals.asr_s += began.elapsed().as_secs_f64();
            let began = Instant::now();
            for frame in clip.samples.as_chunks::<VAD_FRAME>().0 {
                vad.probability(frame);
            }
            totals.vad_s += began.elapsed().as_secs_f64();
            totals.audio_s += audio_s;
            let (errors, words) = word_errors(&clip.reference, &text);
            totals.errors += errors;
            totals.words += words;
        }
        println!(
            "{:<4} {:3} clips {:7.1} s audio  RTF {:.3}  WER {:5.1} %  VAD RTF {:.4}",
            language.name,
            language.clips.len(),
            totals.audio_s,
            totals.rtf(),
            totals.wer(),
            totals.vad_s / totals.audio_s,
        );
        all.add(&totals);
    }
    let (_, rss_peak) = usage();
    let rss_now = resident();
    let mb = |bytes: u64| bytes as f64 / 1_048_576.0;
    println!(
        "all  {:7.1} s audio  RTF {:.3}  WER {:5.1} %  VAD RTF {:.4}",
        all.audio_s,
        all.rtf(),
        all.wer(),
        all.vad_s / all.audio_s,
    );
    println!(
        "memory: {:.0} MB resident at the end, {:.0} MB peak ({:.0} MB before the model, {:.0} MB peak right after loading it)",
        mb(rss_now),
        mb(rss_peak),
        mb(rss_before),
        mb(rss_loaded)
    );
    let mut over = Vec::new();
    if all.rtf() > RTF_BUDGET {
        over.push(format!("RTF {:.3} is over {RTF_BUDGET}", all.rtf()));
    }
    if mb(rss_peak) > RAM_BUDGET_MB {
        over.push(format!(
            "memory {:.0} MB is over {RAM_BUDGET_MB:.0} MB",
            mb(rss_peak)
        ));
    }
    if !over.is_empty() {
        bail!("over budget: {}", over.join(", "));
    }
    println!("within budget: RTF at most {RTF_BUDGET}, memory under {RAM_BUDGET_MB:.0} MB");
    Ok(())
}

/// Resident size in bytes right now. The peak from `getrusage` also counts memory that is freed.
fn resident() -> u64 {
    std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .ok()
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .and_then(|text| text.trim().parse::<u64>().ok())
        .map_or(0, |kilobytes| kilobytes * 1024)
}

fn load_fixtures(dir: &Path) -> Result<Vec<Language>> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Ok(Vec::new());
    };
    let mut folders: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.is_dir())
        .collect();
    folders.sort();
    let mut languages = Vec::new();
    for folder in folders {
        let mut wavs: Vec<PathBuf> = fs::read_dir(&folder)?
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| path.extension().is_some_and(|ext| ext == "wav"))
            .collect();
        wavs.sort();
        let mut clips = Vec::new();
        for wav in wavs {
            let samples = read_wav(&wav).with_context(|| format!("reading {}", wav.display()))?;
            let reference = fs::read_to_string(wav.with_extension("txt"))
                .with_context(|| format!("reading the reference for {}", wav.display()))?;
            clips.push(Clip { samples, reference });
        }
        if !clips.is_empty() {
            let name = folder
                .file_name()
                .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
            languages.push(Language { name, clips });
        }
    }
    Ok(languages)
}

/// 16 kHz mono 16 bit PCM, which is what the fetch script writes.
fn read_wav(path: &Path) -> Result<Vec<f32>> {
    let bytes = fs::read(path)?;
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        bail!("not a WAV file");
    }
    let mut at = 12;
    let mut format_ok = false;
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let size = u32::from_le_bytes([bytes[at + 4], bytes[at + 5], bytes[at + 6], bytes[at + 7]])
            as usize;
        let body = &bytes[at + 8..(at + 8 + size).min(bytes.len())];
        match id {
            b"fmt " if body.len() >= 16 => {
                let number = |i: usize| u16::from_le_bytes([body[i], body[i + 1]]);
                let rate = u32::from_le_bytes([body[4], body[5], body[6], body[7]]);
                // afconvert writes the extensible form, with the real format in the sub-format field.
                let pcm =
                    number(0) == 1 || (number(0) == 0xFFFE && body.len() >= 26 && number(24) == 1);
                format_ok = pcm && number(2) == 1 && number(14) == 16 && rate == RATE;
            }
            b"data" => {
                if !format_ok {
                    bail!("expected 16 kHz mono 16 bit PCM");
                }
                return Ok(body
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|&pair| f32::from(i16::from_le_bytes(pair)) / 32768.0)
                    .collect());
            }
            _ => {}
        }
        at += 8 + size + size % 2;
    }
    bail!("no audio in the file")
}

fn words(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !(c.is_alphanumeric() || c == '\''))
        .filter(|word| !word.is_empty())
        .map(str::to_string)
        .collect()
}

/// Word-level edit distance between the reference and the heard text, and the reference length.
fn word_errors(reference: &str, heard: &str) -> (usize, usize) {
    let (reference, heard) = (words(reference), words(heard));
    let mut row: Vec<usize> = (0..=heard.len()).collect();
    for (i, expected) in reference.iter().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, got) in heard.iter().enumerate() {
            let above = row[j + 1];
            row[j + 1] = (diagonal + usize::from(expected != got))
                .min(above + 1)
                .min(row[j] + 1);
            diagonal = above;
        }
    }
    (row[heard.len()], reference.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn word_errors_count_substitutions_insertions_and_deletions() {
        assert_eq!(word_errors("the cat sat", "The cat, sat."), (0, 3));
        assert_eq!(word_errors("the cat sat", "the bat sat"), (1, 3));
        assert_eq!(word_errors("the cat sat", "the cat sat down"), (1, 3));
        assert_eq!(word_errors("the cat sat", "the sat"), (1, 3));
        assert_eq!(word_errors("Straße zu", "strasse zu"), (1, 2));
    }

    #[test]
    fn a_wav_reads_back_as_floats_and_other_formats_are_refused() {
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF\0\0\0\0WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        for field in [1u16, 1] {
            wav.extend_from_slice(&field.to_le_bytes());
        }
        wav.extend_from_slice(&RATE.to_le_bytes());
        wav.extend_from_slice(&(RATE * 2).to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&4u32.to_le_bytes());
        wav.extend_from_slice(&16384i16.to_le_bytes());
        wav.extend_from_slice(&(-32768i16).to_le_bytes());
        let dir = std::env::temp_dir().join(format!("sextant-wav-{}", std::process::id()));
        fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("a.wav");
        fs::write(&path, &wav).expect("write");
        assert_eq!(read_wav(&path).expect("reads"), vec![0.5, -1.0]);
        wav[24..28].copy_from_slice(&44_100u32.to_le_bytes());
        fs::write(&path, &wav).expect("write");
        assert!(read_wav(&path).is_err());
        let _ = fs::remove_dir_all(&dir);
    }
}
