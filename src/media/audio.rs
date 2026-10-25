//! Audio waveform and processing utilities.
//!
//! Extracts PCM samples via FFmpeg, calculates waveform peaks, detects silence,
//! and renders authentic waveform strips matching the reference design.

use std::ffi::OsString;
use std::path::Path;

use image::{Rgba, RgbaImage};

use crate::media::locator::Tools;
use crate::media::runner::{ProcessRunner, SystemRunner};

/// The background color for waveform tracks (#ECECEA, Tokens.elevated).
pub const WAVEFORM_BG: Rgba<u8> = Rgba([236, 236, 234, 255]);

/// The orange accent color for waveform bars and centerline (#FF5A1F, Tokens.accent).
pub const WAVEFORM_ORANGE: Rgba<u8> = Rgba([255, 90, 31, 255]);

/// Extracts 8000 Hz 16-bit mono PCM samples from an audio or video file.
pub fn pcm_samples(tools: &Tools, input: &Path) -> Option<(Vec<i16>, u32)> {
    let arguments: Vec<OsString> = [
        "-hide_banner",
        "-nostdin",
        "-y",
        "-i",
    ]
    .iter()
    .map(|s| OsString::from(*s))
    .chain([input.as_os_str().to_os_string()])
    .chain(
        [
            "-vn",
            "-ac",
            "1",
            "-ar",
            "8000",
            "-c:a",
            "pcm_s16le",
            "-f",
            "wav",
            "pipe:1",
        ]
        .iter()
        .map(|s| OsString::from(*s)),
    )
    .collect();

    let run = SystemRunner.run(&tools.ffmpeg, &arguments, None).ok()?;
    if !run.succeeded() {
        return None;
    }
    parse_wav_samples(&run.stdout)
}

/// Raw decoded PCM audio data with sample rate and channel count.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PcmAudio {
    pub samples: Vec<i16>,
    pub sample_rate: u32,
    pub channels: u16,
}

/// Parses a WAV byte stream into 16-bit PCM audio samples, sample rate, and channel count.
pub fn parse_wav_audio(bytes: &[u8]) -> Option<PcmAudio> {
    if bytes.len() < 44 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return None;
    }

    let mut sample_rate = 8000u32;
    let mut channels = 1u16;
    let mut offset = 12;
    while offset + 8 <= bytes.len() {
        let chunk_id = &bytes[offset..offset + 4];
        let chunk_size = u32::from_le_bytes([
            bytes[offset + 4],
            bytes[offset + 5],
            bytes[offset + 6],
            bytes[offset + 7],
        ]) as usize;

        if chunk_id == b"fmt " && offset + 8 + chunk_size.min(16) <= bytes.len() {
            let fmt_bytes = &bytes[offset + 8..];
            if fmt_bytes.len() >= 8 {
                channels = u16::from_le_bytes([fmt_bytes[2], fmt_bytes[3]]);
                sample_rate = u32::from_le_bytes([
                    fmt_bytes[4],
                    fmt_bytes[5],
                    fmt_bytes[6],
                    fmt_bytes[7],
                ]);
            }
        } else if chunk_id == b"data" {
            let data_start = offset + 8;
            let data_end = (data_start + chunk_size).min(bytes.len());
            let data = &bytes[data_start..data_end];
            let samples: Vec<i16> = data
                .chunks_exact(2)
                .map(|pair| i16::from_le_bytes([pair[0], pair[1]]))
                .collect();
            return Some(PcmAudio {
                samples,
                sample_rate,
                channels,
            });
        }

        offset += 8 + chunk_size;
        if chunk_size % 2 == 1 {
            offset += 1;
        }
    }

    None
}

/// Parses a WAV byte stream into 16-bit PCM samples and sample rate.
pub fn parse_wav_samples(bytes: &[u8]) -> Option<(Vec<i16>, u32)> {
    parse_wav_audio(bytes).map(|pcm| (pcm.samples, pcm.sample_rate))
}

/// Decodes an audio file to raw 16-bit PCM (keeping its native channels and sample rate) via FFmpeg.
pub fn decode_full_pcm(tools: &Tools, input: &Path) -> Option<PcmAudio> {
    let arguments: Vec<OsString> = [
        "-hide_banner",
        "-nostdin",
        "-y",
        "-i",
    ]
    .iter()
    .map(|s| OsString::from(*s))
    .chain([input.as_os_str().to_os_string()])
    .chain(
        [
            "-vn",
            "-c:a",
            "pcm_s16le",
            "-f",
            "wav",
            "pipe:1",
        ]
        .iter()
        .map(|s| OsString::from(*s)),
    )
    .collect();

    let run = SystemRunner.run(&tools.ffmpeg, &arguments, None).ok()?;
    if !run.succeeded() {
        return None;
    }
    parse_wav_audio(&run.stdout)
}

/// Encodes 16-bit PCM audio samples into a standard RIFF/WAVE byte buffer.
pub fn encode_wav_bytes(samples: &[i16], sample_rate: u32, channels: u16) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let ch = channels.max(1);
    let byte_rate = sample_rate * (ch as u32) * 2;
    let block_align = ch * 2;
    let mut wav = Vec::with_capacity(44 + samples.len() * 2);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_len).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&ch.to_le_bytes());
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    wav.extend_from_slice(&byte_rate.to_le_bytes());
    wav.extend_from_slice(&block_align.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes()); // 16 bits per sample
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    for &sample in samples {
        wav.extend_from_slice(&sample.to_le_bytes());
    }
    wav
}

/// Normalizes 16-bit PCM samples to a target true peak in dBTP.
///
/// Matches Open Tangerine's Swift peak normalization formula:
/// `targetLinear = 10^(truePeak / 20)`
/// `gain = min(4.0, max(0.1, targetLinear / max(peak, 0.0001)))`
/// Returns the normalized samples and the linear gain multiplier.
pub fn normalize_samples(samples: &[i16], true_peak_dbtp: f64) -> (Vec<i16>, f64) {
    if samples.is_empty() {
        return (Vec::new(), 1.0);
    }
    let max_abs = samples.iter().map(|&s| (s as i32).abs()).max().unwrap_or(0);
    let peak = (max_abs as f64) / 32767.0;
    let target_linear = 10.0f64.powf(true_peak_dbtp / 20.0);
    let gain = (target_linear / peak.max(0.0001)).clamp(0.1, 4.0);

    let normalized: Vec<i16> = samples
        .iter()
        .map(|&s| {
            let val = (s as f64 * gain).round();
            val.clamp(i16::MIN as f64, i16::MAX as f64) as i16
        })
        .collect();

    (normalized, gain)
}

/// Groups audio samples into `buckets` and returns the normalized peak amplitude (0.0..=1.0) of each bucket.
pub fn waveform_peaks(samples: &[i16], buckets: usize) -> Vec<f32> {
    if buckets == 0 {
        return Vec::new();
    }
    if samples.is_empty() {
        return vec![0.0; buckets];
    }

    let per_bucket = (samples.len() as f64 / buckets as f64).max(1.0);
    let mut peaks = Vec::with_capacity(buckets);

    for b in 0..buckets {
        let start = (b as f64 * per_bucket) as usize;
        let end = (((b + 1) as f64 * per_bucket) as usize).min(samples.len());
        if start >= samples.len() || start >= end {
            peaks.push(0.0);
            continue;
        }

        let mut max_abs: i32 = 0;
        for &sample in &samples[start..end] {
            let abs_val = (sample as i32).abs();
            if abs_val > max_abs {
                max_abs = abs_val;
            }
        }

        let normalized = (max_abs as f32 / 32768.0).clamp(0.0, 1.0);
        peaks.push(normalized);
    }

    peaks
}

/// Detects leading and trailing silence in the samples, returning `(leading_seconds, trailing_seconds)`.
///
/// Matches the reference Swift `AudioOps.detectSilence` with a normalized threshold (default 0.01).
pub fn detect_silence(samples: &[i16], sample_rate: u32, threshold: f32) -> Option<(f64, f64)> {
    if samples.is_empty() || sample_rate == 0 {
        return None;
    }

    let threshold_val = ((threshold.clamp(0.0, 1.0) * 32768.0) as i32).max(1);
    let mut first = None;
    let mut last = None;

    for (i, &sample) in samples.iter().enumerate() {
        if (sample as i32).abs() >= threshold_val {
            if first.is_none() {
                first = Some(i);
            }
            last = Some(i);
        }
    }

    let first = first?;
    let last = last?;
    let leading = first as f64 / sample_rate as f64;
    let trailing = (last + 1) as f64 / sample_rate as f64;
    Some((leading, trailing))
}

/// Renders a waveform strip with orange vertical bars and a dotted centerline on an elevated background.
pub fn render_waveform_strip(peaks: &[f32], width: u32, height: u32) -> RgbaImage {
    let mut image = RgbaImage::from_pixel(width.max(1), height.max(1), WAVEFORM_BG);
    let w = image.width();
    let h = image.height();
    let cy = h / 2;

    // Draw dotted centerline across the full width
    for x in 0..w {
        if (x / 2) % 2 == 0 {
            image.put_pixel(x, cy, WAVEFORM_ORANGE);
        }
    }

    if peaks.is_empty() {
        return image;
    }

    // Draw vertical bars for each peak
    let count = peaks.len();
    for (i, &peak) in peaks.iter().enumerate() {
        if peak < 0.02 {
            continue;
        }

        let center_x = (i as f32 + 0.5) / count as f32 * w as f32;
        let bar_h = (peak * (h as f32 / 2.0 - 4.0)).round() as u32;
        let bar_h = bar_h.max(2);

        let y_top = cy.saturating_sub(bar_h);
        let y_bottom = (cy + bar_h).min(h - 1);

        let bx_start = (center_x - 1.0).max(0.0) as u32;
        let bx_end = (center_x + 1.0).min((w - 1) as f32) as u32;

        for bx in bx_start..=bx_end {
            for y in y_top..=y_bottom {
                image.put_pixel(bx, y, WAVEFORM_ORANGE);
            }
        }
    }

    image
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_wav_samples_reads_pcm_correctly() {
        // Build minimal valid WAV: 100 samples
        let sample_rate = 8000u32;
        let sample_count = 100u32;
        let data_len = sample_count * 2;
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + data_len).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
        wav.extend_from_slice(&1u16.to_le_bytes()); // mono
        wav.extend_from_slice(&sample_rate.to_le_bytes());
        wav.extend_from_slice(&(sample_rate * 2).to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&data_len.to_le_bytes());
        for i in 0..sample_count {
            let val: i16 = if i < 50 { 16384 } else { -8192 };
            wav.extend_from_slice(&val.to_le_bytes());
        }

        let (samples, rate) = parse_wav_samples(&wav).expect("valid wav");
        assert_eq!(rate, 8000);
        assert_eq!(samples.len(), 100);
        assert_eq!(samples[0], 16384);
        assert_eq!(samples[60], -8192);
    }

    #[test]
    fn waveform_peaks_computes_normalized_amplitudes() {
        let mut samples = vec![0i16; 1000];
        // Set first bucket peak to ~0.5 (16384), second bucket to ~1.0 (32767)
        for s in &mut samples[0..500] {
            *s = 16384;
        }
        for s in &mut samples[500..1000] {
            *s = 32767;
        }

        let peaks = waveform_peaks(&samples, 2);
        assert_eq!(peaks.len(), 2);
        assert!((peaks[0] - 0.5).abs() < 0.01);
        assert!((peaks[1] - 1.0).abs() < 0.01);
    }

    #[test]
    fn detect_silence_finds_active_range() {
        let rate = 8000u32;
        let mut samples = vec![0i16; 8000]; // 1.0 second
        // Active from 0.2s (sample 1600) to 0.8s (sample 6400)
        for s in &mut samples[1600..6400] {
            *s = 10000;
        }

        let (lead, trail) = detect_silence(&samples, rate, 0.01).expect("silence detected");
        assert!((lead - 0.2).abs() < 0.001, "lead: {lead}");
        assert!((trail - 0.8).abs() < 0.001, "trail: {trail}");
    }

    #[test]
    fn render_waveform_strip_produces_correct_image_dimensions() {
        let peaks = vec![0.1, 0.5, 0.8, 0.3];
        let image = render_waveform_strip(&peaks, 372, 72);
        assert_eq!(image.width(), 372);
        assert_eq!(image.height(), 72);
        // Centerline pixel at (0, 36) has orange dot
        assert_eq!(*image.get_pixel(0, 36), WAVEFORM_ORANGE);
        // Background at top left is elevated
        assert_eq!(*image.get_pixel(0, 0), WAVEFORM_BG);
    }

    #[test]
    fn encode_wav_bytes_roundtrips_through_parse() {
        let samples = vec![1000i16, -2000, 3000, -4000, 5000];
        let bytes = encode_wav_bytes(&samples, 44100, 1);
        let parsed = parse_wav_audio(&bytes).expect("valid parsed wav");
        assert_eq!(parsed.sample_rate, 44100);
        assert_eq!(parsed.channels, 1);
        assert_eq!(parsed.samples, samples);
    }

    #[test]
    fn normalize_samples_scales_to_target_true_peak() {
        // Peak is 16384 (~0.5000)
        let samples = vec![0i16, 8192, 16384, -16384, -8192];
        // Target -1.5 dBTP -> 10^(-1.5/20) ~ 0.8414
        let (norm, gain) = normalize_samples(&samples, -1.5);
        assert!((gain - 1.6827).abs() < 0.01, "gain was {gain}");
        let new_peak = norm.iter().map(|&s| (s as i32).abs()).max().unwrap() as f64 / 32767.0;
        assert!((new_peak - 0.8414).abs() < 0.01, "new peak was {new_peak}");
    }

    #[test]
    fn normalize_samples_handles_silent_audio_safely() {
        let silence = vec![0i16; 100];
        let (norm, gain) = normalize_samples(&silence, -1.5);
        assert_eq!(norm.len(), 100);
        assert!(norm.iter().all(|&s| s == 0));
        assert_eq!(gain, 4.0); // clamped to max gain
    }
}
