use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use crate::diagnostics::{self, Level};
use crate::media::manifest::Manifest;
use crate::media::publish::Publisher;
use crate::media::runner::ProcessRunner;
use crate::radial::execution::{Failure, Outcome, RedactStyle, Request};
use super::{metadata_final_names, staging_name, SweepOf, ToolError, Tools};
impl<P: Publisher> Tools<P> {
    ///
    /// A redaction without areas is not a redaction: the handler says so
    /// rather than covering nothing and calling the file changed.
    pub(super) fn run_redact_image(&self, request: &Request) -> Outcome {
        if request.settings.redact_areas.is_empty() {
            return Outcome::Failed {
                reason: Failure::InvalidInput("there are no redaction areas".to_string()),
            };
        }

        let manifest = Manifest::get();
        let mut outputs = Vec::new();
        let mut failed = 0usize;
        let mut first_reason: Option<String> = None;
        for input in &request.inputs {
            let result = self.redact_image_one(input, &manifest, &request.settings);
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

    /// Covers the drawn rectangles of one picture.
    pub(super) fn redact_image_one(
        &self,
        input: &Path,
        manifest: &Manifest,
        settings: &crate::radial::execution::ToolSettings,
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
        if crate::media::probe::kind_of(&source.format) != Some(crate::media::probe::Kind::Image) {
            return Err(ToolError::UnsupportedKind);
        }
        let picture = image::open(input).map_err(|_| ToolError::ReadError)?;
        let covered = redact_picture(picture, settings);
        let temporary = staging_name(input, &format!("redact.{source_ext}"));
        let _sweep = SweepOf(temporary.clone());
        covered.save(&temporary).map_err(|_| ToolError::WriteError)?;

        let finals = metadata_final_names(input, "redacted", &source_ext);
        self.publish(&finals, &temporary)
    }

    /// Covers the parts of videos the window drew on.
    ///
    /// The rectangles go to the conversion program as a filter graph:
    /// a solid box is drawn, a smeared part is the crop made small and
    /// laid back large (the program carries no blur of its own — this
    /// build has none), a pixelated part is the crop made small and laid
    /// back large in squares. Every rectangle shows for its own stretch
    /// of time, told with `between(t, start, end)`; a rectangle given no
    /// stretch covers the whole way through. The copy is made again in
    /// the container it came in, at a constant picture quality — a
    /// redaction is not about the size.
    pub(super) fn run_redact_video(&self, request: &Request, runner: &dyn ProcessRunner) -> Outcome {
        if request.settings.redact_areas.is_empty() {
            return Outcome::Failed {
                reason: Failure::InvalidInput("there are no redaction areas".to_string()),
            };
        }

        let manifest = Manifest::get();
        let mut outputs = Vec::new();
        let mut failed = 0usize;
        let mut first_reason: Option<String> = None;
        for input in &request.inputs {
            let result = self.redact_video_one(
                input,
                &manifest,
                runner,
                &request.settings,
            );
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

    /// Covers the drawn rectangles of one video.
    pub(super) fn redact_video_one(
        &self,
        input: &Path,
        manifest: &Manifest,
        runner: &dyn ProcessRunner,
        settings: &crate::radial::execution::ToolSettings,
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
        if crate::media::probe::kind_of(&source.format) != Some(crate::media::probe::Kind::Video) {
            return Err(ToolError::UnsupportedKind);
        }

        let (width, height, _fps) =
            crate::media::frames::shape_of(&self.ffmpeg_tools, input).ok_or(ToolError::ReadError)?;
        let graph = redact_video_filter(settings, width, height)?;
        let temporary = staging_name(input, &format!("redact.{source_ext}"));
        let _sweep = SweepOf(temporary.clone());
        let arguments = redact_video_args(input, source, &temporary, &graph);

        let finals = metadata_final_names(input, "redacted", &source_ext);
        self.finish(&arguments, source, &temporary, &finals, runner)
    }

    /// Bleeps over the stretches of sounds the window marked.
    ///
    /// The marks are said in milliseconds: every one of them becomes a
    /// span of samples where the original is dropped and a tone takes
    /// its place — the bleep does not hide under the sound, it stands
    /// over it. An overlap shows the last bleep laid, the way covers
    /// stand on a picture. The tone sings at the frequency the settings
    /// ask, and everything outside the marks is carried through for
    /// nothing to hear the difference.
    pub(super) fn run_bleep(&self, request: &Request, runner: &dyn ProcessRunner) -> Outcome {
        if request.settings.bleep_ranges.is_empty() {
            return Outcome::Failed {
                reason: Failure::InvalidInput("there are no bleep ranges".to_string()),
            };
        }

        let manifest = Manifest::get();
        let mut outputs = Vec::new();
        let mut failed = 0usize;
        let mut first_reason: Option<String> = None;
        for input in &request.inputs {
            let result = self.bleep_one(input, &manifest, runner, &request.settings);
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

    /// Bleeps over the marked stretches of one sound, the way the
    /// volume tool works: read as numbers, laid over here, written back
    /// into the container it came in.
    pub(super) fn bleep_one(
        &self,
        input: &Path,
        manifest: &Manifest,
        runner: &dyn ProcessRunner,
        settings: &crate::radial::execution::ToolSettings,
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

        if crate::media::probe::kind_of(&source.format) != Some(crate::media::probe::Kind::Audio) {
            return Err(ToolError::UnsupportedKind);
        }

        let pcm = crate::media::audio::decode_full_pcm(&self.ffmpeg_tools, input)
            .ok_or(ToolError::ReadError)?;

        let mut samples = pcm.samples;
        let channels = pcm.channels.max(1) as usize;
        let frames = samples.len() / channels;
        for range in &settings.bleep_ranges {
            let (start, end) = range.frame_span(frames, pcm.sample_rate);
            bleep_over(
                &mut samples,
                pcm.channels,
                start,
                end,
                pcm.sample_rate,
                settings.bleep_frequency_hz(),
            );
        }

        let temporary = staging_name(input, &format!("{source_ext}.part"));
        let _sweep = SweepOf(temporary.clone());

        if source_ext == "wav" {
            let wav_bytes = crate::media::audio::encode_wav_bytes(
                &samples,
                pcm.sample_rate,
                pcm.channels,
            );
            fs::write(&temporary, wav_bytes).map_err(|_| ToolError::WriteError)?;
        } else {
            let staging_wav = staging_name(input, "staging_bleep.wav");
            let _sweep_wav = SweepOf(staging_wav.clone());
            let wav_bytes = crate::media::audio::encode_wav_bytes(
                &samples,
                pcm.sample_rate,
                pcm.channels,
            );
            fs::write(&staging_wav, wav_bytes).map_err(|_| ToolError::WriteError)?;

            let mut args = vec![
                OsString::from("-hide_banner"),
                OsString::from("-nostdin"),
                OsString::from("-y"),
                OsString::from("-i"),
                staging_wav.as_os_str().to_os_string(),
                OsString::from("-vn"),
                OsString::from("-c:a"),
                OsString::from(&source.write.encoder),
            ];
            if !source.write.muxer.is_empty() {
                args.push(OsString::from("-f"));
                args.push(OsString::from(&source.write.muxer));
            }
            args.push(temporary.as_os_str().to_os_string());

            let run_res = runner.run(&self.ffmpeg_tools.ffmpeg, &args, None);
            let _ = fs::remove_file(&staging_wav);

            match run_res {
                Ok(status) if status.succeeded() => {}
                _ => {
                    let _ = fs::remove_file(&temporary);
                    return Err(ToolError::ToolFailed);
                }
            }
        }

        let finals = metadata_final_names(input, "bleeped", &source_ext);
        self.publish(&finals, &temporary)
    }

}

/// Lays every covered rectangle of the settings on a picture's own pixels.
///
/// The rectangles land through their fractions of the picture's size, and
/// what hangs off an edge is cut away inside the picture. Areas paint in
/// the order they stand in the settings, so an overlap shows the last one
/// that was laid — the order the reference's list paints in, too.
pub(crate) fn redact_picture(
    picture: image::DynamicImage,
    settings: &crate::radial::execution::ToolSettings,
) -> image::DynamicImage {
    if settings.redact_areas.is_empty() {
        return picture;
    }
    let mut buf = picture.to_rgba8();
    let (width, height) = (buf.width(), buf.height());
    let effect = settings.redaction_effect(width.min(height));
    let color = settings.redaction_color();
    for area in &settings.redact_areas {
        let (x, y, w, h) = area.rect_on(width, height);
        if w == 0 || h == 0 {
            continue;
        }
        match settings.redaction_style() {
            RedactStyle::Solid => {
                let box_ = image::ImageBuffer::from_pixel(
                    w,
                    h,
                    image::Rgba([color[0], color[1], color[2], 255]),
                );
                paste(&mut buf, &box_, x, y);
            }
            RedactStyle::Blur => {
                let part = image::imageops::crop(&mut buf, x, y, w, h).to_image();
                let smeared = image::imageops::blur(&part, effect as f32);
                paste(&mut buf, &smeared, x, y);
            }
            RedactStyle::Pixelate => {
                let part = image::imageops::crop(&mut buf, x, y, w, h).to_image();
                let block = effect.max(2.0) as u32;
                let small_w = ((w as f64 / block as f64).ceil() as u32).max(1);
                let small_h = ((h as f64 / block as f64).ceil() as u32).max(1);
                let small = image::imageops::resize(
                    &part,
                    small_w,
                    small_h,
                    image::imageops::FilterType::Triangle,
                );
                let squares =
                    image::imageops::resize(&small, w, h, image::imageops::FilterType::Nearest);
                paste(&mut buf, &squares, x, y);
            }
        }
    }
    image::DynamicImage::ImageRgba8(buf)
}

/// Writes a patch over part of a buffer, pixels exactly as they are.
pub(crate) fn paste(
    buf: &mut image::ImageBuffer<image::Rgba<u8>, Vec<u8>>,
    patch: &image::ImageBuffer<image::Rgba<u8>, Vec<u8>>,
    at_x: u32,
    at_y: u32,
) {
    for dy in 0..patch.height() {
        for dx in 0..patch.width() {
            buf.put_pixel(at_x + dx, at_y + dy, *patch.get_pixel(dx, dy));
        }
    }
}

/// How loud the bleep tone sings, against the top of the sound: half.
///
/// The reference mixed its tone at no level that was written down; half
/// of full scale is heard plainly over speech without clipping against
/// a loud master, and the choice stands recorded with the notes.
const BLEEP_TONE_AMPLITUDE: f64 = 0.5;

/// Lays the bleep over one stretch of a sound.
///
/// Everything the stretch held is dropped first — nothing of the
/// original is left under the tone — and a sine at the asked frequency
/// is written in its place, the same tone in every channel the sound
/// carries. The stretch is said in frames: a frame is one instant of
/// the sound, whatever number of channels it arrives in.
pub fn bleep_over(
    samples: &mut [i16],
    channels: u16,
    start_frame: usize,
    end_frame: usize,
    sample_rate: u32,
    frequency: u32,
) {
    let stride = channels.max(1) as usize;
    for frame in start_frame..end_frame {
        let time = frame as f64 / sample_rate as f64;
        let tone = (BLEEP_TONE_AMPLITUDE
            * (std::f64::consts::TAU * frequency as f64 * time).sin()
            * i16::MAX as f64)
            .round()
            .clamp(i16::MIN as f64, i16::MAX as f64) as i16;
        for channel in 0..stride {
            samples[frame * stride + channel] = tone;
        }
    }
}

/// A stretch of time long enough for anything one video covers: a day.
///
/// A rectangle given no end covers until the picture stops, and the
/// program's `between` wants a number to stop at — this one is a day of
/// seconds, longer than a file a person redacts ever stands.
const ALWAYS_UNTIL_MS: u64 = 86_400_000;

/// Says milliseconds the way the program reads seconds: always three
/// decimals, so `1.5` can never be read as `1.05`.
pub(crate) fn seconds_said(ms: u64) -> String {
    format!("{}.{:03}", ms / 1000, ms % 1000)
}

/// The time rule one rectangle keeps to.
///
/// Nothing at all when the rectangle covers the whole way through;
/// otherwise the program's own between, told where the covering starts
/// and where it stops.
pub(crate) fn redact_window(area: &crate::radial::execution::RedactArea) -> String {
    match (area.start_ms, area.end_ms) {
        (None, None) => String::new(),
        _ => {
            let start = seconds_said(area.start_ms.unwrap_or(0));
            let end = seconds_said(area.end_ms.unwrap_or(ALWAYS_UNTIL_MS));
            format!(":enable='between(t,{start},{end})'")
        }
    }
}

/// The filter graph that lays every covered rectangle on a video.
///
/// The rectangles land in pixels through their fractions, in the order
/// they stand in the settings — an overlap shows the last one laid, the
/// way the picture tool paints them. The program carries no blur of its
/// own in this build, so a smeared part is the crop made small and laid
/// back large, and a pixelated part is the same thing back in squares:
/// laid large with the nearest-neighbor flag, which keeps hard edges.
///
/// The graph ends with `[vout]`, and only stages that cover something
/// of the picture take part. With nothing left, it refuses — the same
/// honesty the picture tool has.
pub(crate) fn redact_video_filter(
    settings: &crate::radial::execution::ToolSettings,
    width: u32,
    height: u32,
) -> Result<String, ToolError> {
    let mut parts: Vec<(
        &crate::radial::execution::RedactArea,
        u32,
        u32,
        u32,
        u32,
    )> = Vec::new();
    for area in &settings.redact_areas {
        let (x, y, w, h) = area.rect_on(width, height);
        if w > 0 && h > 0 {
            parts.push((area, x, y, w, h));
        }
    }
    if parts.is_empty() {
        return Err(ToolError::NothingToDo);
    }

    let color = settings.redaction_color();
    let hex = format!("0x{:02x}{:02x}{:02x}@1", color[0], color[1], color[2]);
    let effect = settings.redaction_effect(width.min(height));
    let mut graph = String::new();
    let mut flowing = String::from("0:v");
    for (stage, part) in parts.iter().enumerate() {
        let (area, x, y, w, h) = *part;
        let window = redact_window(area);
        let last = stage == parts.len() - 1;
        let out = if last { "vout".to_string() } else { format!("r{stage}") };
        match settings.redaction_style() {
            RedactStyle::Solid => {
                graph.push_str(&format!(
                    "[{flowing}]drawbox=x={x}:y={y}:w={w}:h={h}:color={hex}:t=fill{window}[{out}];",
                    flowing = flowing,
                    out = out,
                ));
                if !last {
                    flowing = format!("r{stage}");
                }
            }
            RedactStyle::Blur => {
                // the crop made small and laid back large: a smear by
                // the program's scale, painted over for its own time
                let small_w = (w as f64 / effect).round().max(1.0) as u32;
                let small_h = (h as f64 / effect).round().max(1.0) as u32;
                graph.push_str(&format!(
                    "[{flowing}]split=2[m{stage}][b{stage}];",
                    flowing = flowing,
                ));
                graph.push_str(&format!(
                    "[b{stage}]crop=x={x}:y={y}:w={w}:h={h},scale={small_w}:{small_h},scale={w}:{h}[sm{stage}];"
                ));
                graph.push_str(&format!(
                    "[m{stage}][sm{stage}]overlay=x={x}:y={y}{window}[{out}];",
                    out = out,
                ));
                if !last {
                    flowing = format!("r{stage}");
                }
            }
            RedactStyle::Pixelate => {
                let block = effect.max(2.0);
                let small_w = ((w as f64 / block).ceil() as u32).max(1);
                let small_h = ((h as f64 / block).ceil() as u32).max(1);
                graph.push_str(&format!(
                    "[{flowing}]split=2[m{stage}][b{stage}];",
                    flowing = flowing,
                ));
                graph.push_str(&format!(
                    "[b{stage}]crop=x={x}:y={y}:w={w}:h={h},scale={small_w}:{small_h},scale={w}:{h}:flags=neighbor[px{stage}];"
                ));
                graph.push_str(&format!(
                    "[m{stage}][px{stage}]overlay=x={x}:y={y}{window}[{out}];",
                    out = out,
                ));
                if !last {
                    flowing = format!("r{stage}");
                }
            }
        }
    }
    graph.pop(); // the last statement wants no semicolon after it
    Ok(graph)
}

/// The arguments that make a covered video: the picture made again in
/// its own container at a constant quality, sound carried as sound is
/// carried when a conversion makes a video file.
pub(crate) fn redact_video_args(
    input: &Path,
    source: &crate::media::manifest::FormatSupport,
    output_path: &Path,
    graph: &str,
) -> Vec<OsString> {
    let mut args = vec![
        OsString::from("-hide_banner"),
        OsString::from("-nostdin"),
        OsString::from("-y"),
        OsString::from("-f"),
        OsString::from(&source.read.demuxer),
        OsString::from("-i"),
        input.as_os_str().to_os_string(),
        OsString::from("-filter_complex"),
        OsString::from(graph),
        OsString::from("-map"),
        OsString::from("[vout]"),
        OsString::from("-map"),
        OsString::from("0:a?"),
        OsString::from("-c:v"),
        OsString::from(&source.write.encoder),
        OsString::from("-q:v"),
        OsString::from("5"),
        OsString::from("-c:a"),
        OsString::from("aac"),
        OsString::from("-b:a"),
        OsString::from("128000"),
        OsString::from("-f"),
        OsString::from(&source.write.muxer),
    ];
    args.push(output_path.as_os_str().to_os_string());
    args
}

