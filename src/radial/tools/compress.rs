use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use crate::diagnostics::{self, Level};
use crate::media::locator::Tools as FfmpegTools;
use crate::media::manifest::{FormatSupport, Manifest};
use crate::media::publish::Publisher;
use crate::media::runner::ProcessRunner;
use crate::radial::execution::{Failure, Outcome, Request, ToolSettings};

use super::{final_names, metadata_final_names, muxer_needs_file, staging_name, SweepOf, ToolError, Tools};

impl<P: Publisher> Tools<P> {
    /// Makes files lighter, at the preset and the target size the window carried.
    pub(super) fn run_compress(&self, request: &Request, runner: &dyn ProcessRunner) -> Outcome {
        let manifest = Manifest::get();
        let mut outputs = Vec::new();
        let mut failed = 0usize;
        let mut first_reason: Option<String> = None;
        for input in &request.inputs {
            let result = self.compress_one(input, &manifest, runner, &request.settings);
            match result {
                Ok(path) => outputs.push(path),
                Err(reason) => {
                    failed += 1;
                    let name = reason.name();
                    diagnostics::log(Level::Warn, "action", &format!("a file was not processed: {name}"));
                    if first_reason.is_none() {
                        first_reason = Some(name.to_string());
                    }
                }
            }
        }

        if failed == 0 {
            Outcome::Succeeded { outputs }
        } else {
            let because = match first_reason {
                Some(reason) => format!(": {reason}"),
                None => String::new(),
            };
            Outcome::Failed {
                reason: Failure::Error(format!(
                    "{failed} of the files could not be processed{because}"
                )),
            }
        }
    }

    /// Compresses one file, whichever kind it is.
    pub(super) fn compress_one(
        &self,
        input: &Path,
        manifest: &Manifest,
        runner: &dyn ProcessRunner,
        settings: &ToolSettings,
    ) -> Result<PathBuf, ToolError> {
        let source_ext = input
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        let source = manifest
            .format_of_extension(&source_ext)
            .or_else(|| manifest.format(&source_ext))
            .ok_or(ToolError::UnsupportedKind)?;
        let temporary = staging_name(input, &format!("{source_ext}.part"));
        let _sweep = SweepOf(temporary.clone());

        match crate::media::probe::kind_of(&source.format) {
            Some(crate::media::probe::Kind::Image) => match source.format.as_str() {
                "jpeg" | "png" => {
                    compress_picture(input, &temporary, settings)?;
                    let finals = final_names(input, &source_ext);
                    self.publish(&finals, &temporary)
                }
                "webp" => {
                    match settings.target_bytes {
                        Some(bytes) if bytes > 0 => {
                            search_webp_quality(
                                input,
                                source,
                                &temporary,
                                &self.ffmpeg_tools,
                                runner,
                                bytes,
                            )?;
                            let finals = final_names(input, &source_ext);
                            self.publish(&finals, &temporary)
                        }
                        _ => {
                            let arguments = webp_args(
                                input,
                                source,
                                Some(&temporary),
                                webp_quality(settings.compress_strong),
                            );
                            let finals = final_names(input, &source_ext);
                            self.finish(&arguments, source, &temporary, &finals, runner)
                        }
                    }
                }
                _ => Err(ToolError::UnsupportedKind),
            },
            Some(crate::media::probe::Kind::Video) => {
                let duration = crate::media::frames::duration_of(&self.ffmpeg_tools, input)
                    .ok_or(ToolError::ReadError)?;
                let (width, height, fps) =
                    crate::media::frames::shape_of(&self.ffmpeg_tools, input)
                        .ok_or(ToolError::ReadError)?;
                let weight = fs::metadata(input).map(|meta| meta.len()).unwrap_or(0);
                let picture_bits = video_bitrate(
                    weight,
                    duration,
                    settings.compress_strong,
                    settings.target_bytes,
                );
                let sound_bits = audio_bitrate(settings.compress_strong);
                let filter = compress_video_filter(width, height, fps, settings);
                let arguments = video_compress_args(
                    input,
                    source,
                    Some(&temporary),
                    filter,
                    picture_bits,
                    sound_bits,
                );
                let finals = final_names(input, &source_ext);
                self.finish(&arguments, source, &temporary, &finals, runner)
            }
            Some(crate::media::probe::Kind::Audio) => {
                let duration = crate::media::frames::duration_of(&self.ffmpeg_tools, input)
                    .ok_or(ToolError::ReadError)?;
                let bits = match settings.target_bytes {
                    Some(bytes) if bytes > 0 => audio_bitrate_for_target(bytes, duration),
                    _ => audio_bitrate(settings.compress_strong),
                };
                let arguments = audio_compress_args(input, source, Some(&temporary), bits);
                let finals = final_names(input, &source_ext);
                self.finish(&arguments, source, &temporary, &finals, runner)
            }
            _ => Err(ToolError::UnsupportedKind),
        }
    }

    /// Edits or strips metadata on the files in the request.
    pub(super) fn run_metadata(&self, request: &Request, runner: &dyn ProcessRunner) -> Outcome {
        let manifest = Manifest::get();
        let mut outputs = Vec::new();
        let mut failed = 0usize;
        let mut first_reason: Option<String> = None;
        for input in &request.inputs {
            let result = self.metadata_one(input, &manifest, runner, &request.settings);
            match result {
                Ok(path) => outputs.push(path),
                Err(reason) => {
                    failed += 1;
                    let name = reason.name();
                    diagnostics::log(Level::Warn, "action", &format!("a file was not processed: {name}"));
                    if first_reason.is_none() {
                        first_reason = Some(name.to_string());
                    }
                }
            }
        }

        if failed == 0 {
            Outcome::Succeeded { outputs }
        } else {
            let because = match first_reason {
                Some(reason) => format!(": {reason}"),
                None => String::new(),
            };
            Outcome::Failed {
                reason: Failure::Error(format!(
                    "{failed} of the files could not be processed{because}"
                )),
            }
        }
    }

    /// Edits or strips metadata on one file.
    pub(super) fn metadata_one(
        &self,
        input: &Path,
        manifest: &Manifest,
        runner: &dyn ProcessRunner,
        settings: &ToolSettings,
    ) -> Result<PathBuf, ToolError> {
        let source_ext = input
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        let source = manifest
            .format_of_extension(&source_ext)
            .or_else(|| manifest.format(&source_ext))
            .ok_or(ToolError::UnsupportedKind)?;
        let stem = input
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "file".to_string());
        let temporary = input.with_file_name(format!("{stem}.meta-part.{source_ext}"));

        let suffix = if settings.metadata_remove_all {
            "clean"
        } else {
            "edited"
        };

        if source.format == "pdf" {
            let content = std::fs::read(input).map_err(|_| ToolError::ReadError)?;
            std::fs::write(&temporary, &content).map_err(|_| ToolError::WriteError)?;
            let finals = metadata_final_names(input, suffix, &source_ext);
            return self.publish(&finals, &temporary);
        }

        match crate::media::probe::kind_of(&source.format) {
            Some(crate::media::probe::Kind::Image) => {
                let asks_for_changes =
                    settings.metadata_remove_sensitive || !settings.metadata_tags.is_empty();
                let is_jpeg_family =
                    matches!(source_ext.as_str(), "jpg" | "jpeg" | "jpe" | "jfif");

                let img = image::open(input).map_err(|_| ToolError::ReadError)?;

                if settings.metadata_remove_all || !asks_for_changes {
                    img.save(&temporary).map_err(|_| ToolError::WriteError)?;
                    let finals = metadata_final_names(input, suffix, &source_ext);
                    return self.publish(&finals, &temporary);
                }

                if !is_jpeg_family {
                    return Err(ToolError::NotImplemented);
                }

                let mut fresh = Vec::new();
                img.write_to(&mut std::io::Cursor::new(&mut fresh), image::ImageFormat::Jpeg)
                    .map_err(|_| ToolError::WriteError)?;

                let original = crate::media::metadata::exif_fields(input).unwrap_or_default();
                let edits = crate::media::metadata::edits_from_ffmpeg_keys(&settings.metadata_tags);
                let fields = crate::media::metadata::recompose_exif_fields(
                    original,
                    &edits,
                    settings.metadata_remove_sensitive,
                );
                let written = crate::media::metadata::jpeg_with_exif(&fresh, &fields)
                    .map_err(|_| ToolError::WriteError)?;
                std::fs::write(&temporary, written).map_err(|_| ToolError::WriteError)?;

                let finals = metadata_final_names(input, suffix, &source_ext);
                self.publish(&finals, &temporary)
            }
            Some(crate::media::probe::Kind::Video | crate::media::probe::Kind::Audio) => {
                let arguments = metadata_args(input, &temporary, Some(&source.write.muxer), settings);
                let run = runner
                    .run(&self.ffmpeg_tools.ffmpeg, &arguments, None)
                    .map_err(|_| ToolError::ToolFailed)?;
                if !run.succeeded() {
                    let _ = std::fs::remove_file(&temporary);
                    return Err(ToolError::ToolFailed);
                }
                let finals = metadata_final_names(input, suffix, &source_ext);
                self.publish(&finals, &temporary)
            }
            _ => Err(ToolError::UnsupportedKind),
        }
    }
}

/// The arguments built for FFmpeg to edit or strip metadata.
pub fn metadata_args(
    input: &Path,
    output: &Path,
    muxer: Option<&str>,
    settings: &ToolSettings,
) -> Vec<OsString> {
    let mut args = vec![
        OsString::from("-hide_banner"),
        OsString::from("-nostdin"),
        OsString::from("-y"),
        OsString::from("-i"),
        input.as_os_str().to_os_string(),
        OsString::from("-map"),
        OsString::from("0"),
        OsString::from("-c"),
        OsString::from("copy"),
    ];

    if let Some(muxer) = muxer {
        args.push(OsString::from("-f"));
        args.push(OsString::from(muxer));
    }

    if settings.metadata_remove_all {
        args.push(OsString::from("-map_metadata"));
        args.push(OsString::from("-1"));
        args.push(OsString::from("-map_chapters"));
        args.push(OsString::from("-1"));
    } else {
        args.push(OsString::from("-map_metadata"));
        args.push(OsString::from("0"));
        for (key, val) in &settings.metadata_tags {
            let trimmed = val.trim();
            args.push(OsString::from("-metadata"));
            args.push(OsString::from(format!("{key}={trimmed}")));
        }

        if settings.metadata_remove_sensitive {
            for tag in &["location", "location-eng", "author", "comment", "copyright"] {
                args.push(OsString::from("-metadata"));
                args.push(OsString::from(format!("{tag}=")));
            }
        }
    }

    args.push(output.as_os_str().to_os_string());
    args
}

/// The resize filter for scaling images/videos.
pub fn resize_filter(settings: &ToolSettings) -> String {
    match (settings.width, settings.height) {
        (Some(width), Some(height)) => format!("scale={width}:{height}"),
        (Some(width), None) => format!("scale={width}:-1"),
        (None, Some(height)) => format!("scale=-1:{height}"),
        (None, None) => "scale=iw/2:ih/2".to_string(),
    }
}

/// The quality the two presets write a JPEG with.
pub fn jpeg_quality(strong: bool) -> u8 {
    if strong {
        50
    } else {
        72
    }
}

/// The quality the two presets ask of the WebP encoder.
pub fn webp_quality(strong: bool) -> u32 {
    if strong {
        50
    } else {
        72
    }
}

/// The bits a second a compressed sound is written with.
pub fn audio_bitrate(strong: bool) -> u64 {
    if strong {
        64_000
    } else {
        128_000
    }
}

/// The bits a second a compressed video's picture is written with.
pub fn video_bitrate(weight: u64, duration: f64, strong: bool, target: Option<u64>) -> u64 {
    match (target, duration > 0.0) {
        (Some(bytes), true) if bytes > 0 => {
            let total_bits = bytes as f64 * 8.0 * 0.94;
            let sound_bits = audio_bitrate(strong) as f64;
            (total_bits / duration - sound_bits).max(150_000.0) as u64
        }
        (_, true) => {
            let estimated = weight as f64 * 8.0 / duration;
            let share = if strong { 0.35 } else { 0.6 };
            (estimated * share).max(300_000.0) as u64
        }
        _ => 300_000,
    }
}

/// The bits a second a compressed sound takes when a target size was asked.
pub fn audio_bitrate_for_target(target: u64, duration: f64) -> u64 {
    if duration <= 0.0 {
        return 128_000;
    }
    let bits = target as f64 * 8.0 * 0.96 / duration;
    (bits.max(32_000.0).min(320_000.0)) as u64
}

/// The picture filter a compressed video carries, if it carries one.
pub fn compress_video_filter(
    width: u32,
    height: u32,
    fps: f64,
    settings: &ToolSettings,
) -> Option<String> {
    let mut filters: Vec<String> = Vec::new();
    let long_now = width.max(height) as f64;
    if long_now > 0.0 {
        let share = if settings.compress_strong {
            0.75
        } else {
            1.0
        };
        let mut limit = long_now * share;
        if let Some(cap) = settings.max_long_edge {
            if cap > 0 {
                limit = limit.min(cap as f64);
            }
        }
        if limit < long_now - 1.0 {
            let even = |edge: f64| ((edge / 2.0).round() as u32).max(1) * 2;
            if width >= height {
                filters.push(format!("scale={}:{}", even(limit), "-2"));
            } else {
                filters.push(format!("scale=-2:{}", even(limit)));
            }
        }
    }
    if let Some(cap) = settings.max_fps {
        if cap > 0 && fps > cap as f64 + 0.5 {
            filters.push(format!("fps={cap}"));
        }
    }
    if filters.is_empty() {
        None
    } else {
        Some(filters.join(","))
    }
}

/// The arguments that make a video lighter.
pub fn video_compress_args(
    input: &Path,
    source: &FormatSupport,
    output_path: Option<&Path>,
    filter: Option<String>,
    picture_bits: u64,
    sound_bits: u64,
) -> Vec<OsString> {
    let mut args = vec![
        OsString::from("-hide_banner"),
        OsString::from("-nostdin"),
        OsString::from("-y"),
        OsString::from("-f"),
        OsString::from(&source.read.demuxer),
        OsString::from("-i"),
        input.as_os_str().to_os_string(),
    ];
    if let Some(filter) = filter {
        args.push(OsString::from("-vf"));
        args.push(OsString::from(filter));
    }
    args.push(OsString::from("-c:v"));
    args.push(OsString::from(&source.write.encoder));
    args.push(OsString::from("-b:v"));
    args.push(OsString::from(picture_bits.to_string()));
    args.push(OsString::from("-c:a"));
    args.push(OsString::from("aac"));
    args.push(OsString::from("-b:a"));
    args.push(OsString::from(sound_bits.to_string()));
    args.push(OsString::from("-f"));
    args.push(OsString::from(&source.write.muxer));
    match output_path {
        Some(path) => args.push(path.as_os_str().to_os_string()),
        None => args.push(OsString::from("pipe:1")),
    }
    args
}

/// The arguments that make a sound lighter.
pub fn audio_compress_args(
    input: &Path,
    source: &FormatSupport,
    output_path: Option<&Path>,
    bits: u64,
) -> Vec<OsString> {
    let mut args = vec![
        OsString::from("-hide_banner"),
        OsString::from("-nostdin"),
        OsString::from("-y"),
        OsString::from("-f"),
        OsString::from(&source.read.demuxer),
        OsString::from("-i"),
        input.as_os_str().to_os_string(),
        OsString::from("-c:a"),
        OsString::from(&source.write.encoder),
        OsString::from("-b:a"),
        OsString::from(bits.to_string()),
        OsString::from("-f"),
        OsString::from(&source.write.muxer),
    ];
    match output_path {
        Some(path) => args.push(path.as_os_str().to_os_string()),
        None => args.push(OsString::from("pipe:1")),
    }
    args
}

/// The arguments that write a WebP at a quality.
pub fn webp_args(
    input: &Path,
    source: &FormatSupport,
    output_path: Option<&Path>,
    quality: u32,
) -> Vec<OsString> {
    let mut args = vec![
        OsString::from("-hide_banner"),
        OsString::from("-nostdin"),
        OsString::from("-y"),
        OsString::from("-f"),
        OsString::from(&source.read.demuxer),
        OsString::from("-i"),
        input.as_os_str().to_os_string(),
        OsString::from("-c:v"),
        OsString::from(&source.write.encoder),
        OsString::from("-q:v"),
        OsString::from(quality.to_string()),
        OsString::from("-frames:v"),
        OsString::from("1"),
        OsString::from("-f"),
        OsString::from(&source.write.muxer),
    ];
    match output_path {
        Some(path) => args.push(path.as_os_str().to_os_string()),
        None => args.push(OsString::from("pipe:1")),
    }
    args
}

/// Writes a JPEG into the temporary at a quality.
pub fn write_jpeg(
    picture: &image::DynamicImage,
    path: &Path,
    quality: u8,
) -> Result<(), ToolError> {
    let file = fs::File::create(path).map_err(|_| ToolError::WriteError)?;
    let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(file, quality);
    picture
        .write_with_encoder(encoder)
        .map_err(|_| ToolError::WriteError)
}

/// A picture made a share smaller, its shape kept, never nothing.
pub fn scale_by(picture: &image::DynamicImage, share: f64) -> image::DynamicImage {
    let width = ((picture.width() as f64) * share).round().max(1.0) as u32;
    let height = ((picture.height() as f64) * share).round().max(1.0) as u32;
    picture.resize_exact(width, height, image::imageops::FilterType::Lanczos3)
}

/// A picture whose long edge stands at most where it is asked, its shape kept.
pub fn shrink_long_edge(picture: &image::DynamicImage, limit: u32) -> image::DynamicImage {
    let long = picture.width().max(picture.height());
    if long == 0 || limit == 0 || long <= limit {
        return picture.clone();
    }
    picture.resize(limit, limit, image::imageops::FilterType::Lanczos3)
}

/// Makes a picture lighter with the program's own hands.
pub fn compress_picture(
    input: &Path,
    temporary: &Path,
    settings: &ToolSettings,
) -> Result<(), ToolError> {
    let extension = input
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    match extension.as_str() {
        "jpeg" | "jpg" => {
            let picture = image::open(input).map_err(|_| ToolError::ReadError)?;
            match settings.target_bytes {
                Some(bytes) if bytes > 0 => {
                    search_jpeg_quality(picture, temporary, bytes)
                }
                _ => write_jpeg(&picture, temporary, jpeg_quality(settings.compress_strong)),
            }
        }
        "png" => {
            let mut picture = image::open(input).map_err(|_| ToolError::ReadError)?;
            if settings.compress_strong && settings.target_bytes.is_none() {
                let long = picture.width().max(picture.height());
                picture = shrink_long_edge(&picture, (long as f64 * 0.7) as u32);
            }
            match settings.target_bytes {
                Some(bytes) if bytes > 0 => {
                    let mut candidate = picture;
                    let mut fits = false;
                    for _ in 0..12 {
                        candidate
                            .save_with_format(temporary, image::ImageFormat::Png)
                            .map_err(|_| ToolError::WriteError)?;
                        let size = fs::metadata(temporary)
                            .map(|meta| meta.len())
                            .unwrap_or(u64::MAX);
                        if size <= bytes {
                            fits = true;
                            break;
                        }
                        if candidate.width() < 2 || candidate.height() < 2 {
                            break;
                        }
                        candidate = scale_by(&candidate, 0.8);
                    }
                    if fits {
                        Ok(())
                    } else {
                        Err(ToolError::TooSmall)
                    }
                }
                _ => picture
                    .save_with_format(temporary, image::ImageFormat::Png)
                    .map_err(|_| ToolError::WriteError),
            }
        }
        _ => Err(ToolError::UnsupportedKind),
    }
}

/// Searches for the highest JPEG quality whose file fits the target bytes.
pub fn search_jpeg_quality(
    mut candidate: image::DynamicImage,
    temporary: &Path,
    target: u64,
) -> Result<(), ToolError> {
    let mut low = 0.15f64;
    let mut high = 0.95f64;
    let mut best: Option<u8> = None;
    for _ in 0..12 {
        let quality = (low + high) / 2.0;
        let said = (quality * 100.0).round() as u8;
        write_jpeg(&candidate, temporary, said)?;
        let size = fs::metadata(temporary)
            .map(|meta| meta.len())
            .unwrap_or(u64::MAX);
        if size <= target {
            best = Some(said);
            low = quality;
            if (size as f64) > target as f64 * 0.9 {
                break;
            }
        } else {
            high = quality;
        }
        if high - low < 0.03 {
            if best.is_some() {
                break;
            }
            candidate = scale_by(&candidate, 0.8);
            low = 0.15;
            high = 0.95;
            if candidate.width() <= 64 {
                break;
            }
        }
    }
    if let Some(said) = best {
        write_jpeg(&candidate, temporary, said)?;
    }
    Ok(())
}

/// Runs the program once to write a WebP into the temporary at a quality.
pub fn run_webp_once(
    input: &Path,
    source: &FormatSupport,
    temporary: &Path,
    tools: &FfmpegTools,
    runner: &dyn ProcessRunner,
    quality: u32,
) -> Result<u64, ToolError> {
    let arguments = webp_args(input, source, Some(temporary), quality);
    let size = if muxer_needs_file(&source.write.muxer) {
        let result = runner
            .run(&tools.ffmpeg, &arguments, None)
            .map_err(|_| ToolError::ToolFailed)?;
        if !result.succeeded() {
            return Err(ToolError::ToolFailed);
        }
        fs::metadata(temporary).map(|meta| meta.len()).unwrap_or(0)
    } else {
        let mut file = fs::File::create(temporary).map_err(|_| ToolError::WriteError)?;
        let result = runner
            .run(&tools.ffmpeg, &arguments, Some(&mut file))
            .map_err(|_| ToolError::ToolFailed)?;
        drop(file);
        if !result.succeeded() {
            return Err(ToolError::ToolFailed);
        }
        fs::metadata(temporary).map(|meta| meta.len()).unwrap_or(0)
    };
    Ok(size)
}

/// Searches for the WebP quality whose file fits the size that was asked.
pub fn search_webp_quality(
    input: &Path,
    source: &FormatSupport,
    temporary: &Path,
    tools: &FfmpegTools,
    runner: &dyn ProcessRunner,
    target: u64,
) -> Result<(), ToolError> {
    let mut low = 15u32;
    let mut high = 95u32;
    let mut best: Option<u32> = None;
    let mut smallest: Option<u32> = None;
    let mut smallest_size = u64::MAX;
    for _ in 0..12 {
        let quality = (low + high) / 2;
        let size = run_webp_once(input, source, temporary, tools, runner, quality)?;
        if size < smallest_size {
            smallest_size = size;
            smallest = Some(quality);
        }
        if size <= target {
            best = Some(quality);
            low = quality;
            if size > target / 10 * 9 {
                break;
            }
        } else {
            high = quality;
        }
        if high.saturating_sub(low) < 3 {
            break;
        }
    }
    let quality = best.or(smallest).ok_or(ToolError::ToolFailed)?;
    run_webp_once(input, source, temporary, tools, runner, quality)?;
    Ok(())
}
