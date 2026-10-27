use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::diagnostics::{self, Level};
use crate::media::manifest::{FormatSupport, Manifest};
use crate::media::publish::Publisher;
use crate::media::runner::{ProcessRunner, SystemRunner};
use crate::radial::execution::{Failure, Outcome, Request, ToolSettings};

use super::{final_names, metadata_final_names, staging_name, SweepOf, ToolError, Tools};

impl<P: Publisher> Tools<P> {
    /// Cuts a media file down to the chosen rectangle.
    ///
    /// A video goes through the conversion program, where the picture is
    /// cropped and scaled; an image is cut in memory, without the
    /// conversion program at all. A sound has no edges to take off.
    pub(super) fn run_crop(
        &self,
        input: &Path,
        target: &FormatSupport,
        temporary: &Path,
        runner: &dyn ProcessRunner,
        settings: &ToolSettings,
    ) -> Result<PathBuf, ToolError> {
        let source_ext = input
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        match crate::media::probe::kind_of(&source_ext) {
            Some(crate::media::probe::Kind::Video) => {
                let Some((x, y, w, h)) = crop_rect(settings) else {
                    return Err(ToolError::NothingToDo);
                };
                let arguments = crop_args(
                    input,
                    target,
                    Some(temporary),
                    (x, y, w, h),
                    (settings.width, settings.height),
                );
                let finals = final_names(input, target.format.as_str());
                self.finish(&arguments, target, temporary, &finals, runner)
            }
            Some(crate::media::probe::Kind::Image) => {
                crop_image(input, temporary, settings)?;
                let finals = final_names(input, &source_ext);
                self.publish(&finals, temporary)
            }
            _ => Err(ToolError::UnsupportedKind),
        }
    }

    /// Splits a video into several clips at the chosen split points.
    pub(super) fn run_split(&self, request: &Request, runner: &dyn ProcessRunner) -> Outcome {
        if request.settings.points_ms.is_empty() {
            return Outcome::Failed {
                reason: Failure::InvalidInput("there are no split points".to_string()),
            };
        }

        let manifest = Manifest::get();
        let mut outputs = Vec::new();
        let mut failed = 0usize;
        let mut first_reason: Option<String> = None;
        for input in &request.inputs {
            let result = self.split_one(input, &manifest, runner, &request.settings);
            match result {
                Ok(paths) => outputs.extend(paths),
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

    /// Splits one video into its folder of numbered clips.
    pub(super) fn split_one(
        &self,
        input: &Path,
        manifest: &Manifest,
        runner: &dyn ProcessRunner,
        settings: &ToolSettings,
    ) -> Result<Vec<PathBuf>, ToolError> {
        let source_ext = input
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        let source = manifest
            .format_of_extension(&source_ext)
            .or_else(|| manifest.format(&source_ext))
            .ok_or(ToolError::UnsupportedKind)?;

        // The video says how long it stands: the points stand inside it,
        // and the last clip runs to its end.
        let duration = crate::media::frames::duration_of(&self.ffmpeg_tools, input)
            .ok_or(ToolError::ReadError)?;
        let segments = split_segments(&settings.points_ms, duration);
        if segments.is_empty() {
            return Err(ToolError::NothingToDo);
        }

        let stem = input
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_else(|| "clip".to_string());
        let under = input
            .parent()
            .map(|parent| parent.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        let folder = unique_folder(&under, &format!("{stem} clips"))?;

        let mut outputs = Vec::new();
        for (index, (start, end)) in segments.iter().enumerate() {
            let asked = ToolSettings {
                start_ms: Some(*start),
                end_ms: Some(*end),
                ..Default::default()
            };
            let leaf = format!("{} part {}.{source_ext}", stem, index + 1);
            let final_name = folder.join(&leaf);
            let work_path = folder.join(format!("{leaf}.part"));
            let _sweep = SweepOf(work_path.clone());
            let arguments = trim_args(input, source, source, Some(&work_path), &asked);
            let made = self.finish(&arguments, source, &work_path, &[final_name], runner);
            match made {
                Ok(path) => outputs.push(path),
                Err(reason) => return Err(reason),
            }
        }
        Ok(outputs)
    }

    /// Takes the moments the window asked for as pictures, in a folder
    /// of its own beside the original, the way a split's clips stand.
    pub(super) fn run_snapshots(&self, request: &Request, _runner: &dyn ProcessRunner) -> Outcome {
        if request.settings.snapshots_ms.is_empty() {
            return Outcome::Failed {
                reason: Failure::InvalidInput("there are no snapshots".to_string()),
            };
        }

        let manifest = Manifest::get();
        let mut outputs = Vec::new();
        let mut failed = 0usize;
        let mut first_reason: Option<String> = None;
        for input in &request.inputs {
            let result = self.snapshots_one(input, &manifest, &request.settings);
            match result {
                Ok(paths) => outputs.extend(paths),
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

    /// Takes one video's snapshots, each a picture of its own.
    pub(super) fn snapshots_one(
        &self,
        input: &Path,
        manifest: &Manifest,
        settings: &ToolSettings,
    ) -> Result<Vec<PathBuf>, ToolError> {
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

        let duration = crate::media::frames::duration_of(&self.ffmpeg_tools, input)
            .ok_or(ToolError::ReadError)?;
        let mut shots: Vec<u64> = settings
            .snapshots_ms
            .iter()
            .copied()
            .filter(|&ms| (ms as f64) <= duration * 1000.0)
            .collect();
        if shots.is_empty() {
            return Err(ToolError::NothingToDo);
        }
        shots.sort_unstable();

        let stem = input
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_else(|| "clip".to_string());
        let under = input
            .parent()
            .map(|parent| parent.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        let folder = unique_folder(&under, &format!("{stem} frames"))?;

        let mut outputs = Vec::new();
        for (index, ms) in shots.iter().enumerate() {
            let Some(picture) =
                crate::media::frames::frame_at(&self.ffmpeg_tools, input, *ms as f64 / 1000.0)
            else {
                diagnostics::log(
                    Level::Warn,
                    "action",
                    &format!("a frame was not taken at {} ms", ms),
                );
                continue;
            };
            let target = folder.join(format!("{stem} frame {}.png", index + 1));
            picture
                .save(&target)
                .map_err(|_| ToolError::WriteError)?;
            outputs.push(target);
        }
        if outputs.is_empty() {
            return Err(ToolError::ReadError);
        }
        Ok(outputs)
    }

    /// Lays the speed the window asked on the files in the request.
    pub(super) fn run_speed(&self, request: &Request, _runner: &dyn ProcessRunner) -> Outcome {
        let Some(percent) = request.settings.speed_percent else {
            return Outcome::Failed {
                reason: Failure::InvalidInput("there is no speed to run at".to_string()),
            };
        };

        let manifest = Manifest::get();
        let mut outputs = Vec::new();
        let mut failed = 0usize;
        let mut first_reason: Option<String> = None;
        for input in &request.inputs {
            let result = self.speed_one(input, &manifest, percent);
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

    /// Retimes one video, whole frames taken and said again.
    pub(super) fn speed_one(
        &self,
        input: &Path,
        manifest: &Manifest,
        percent: u32,
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

        let (width, height, rate) = crate::media::frames::shape_of(&self.ffmpeg_tools, input)
            .ok_or(ToolError::ReadError)?;
        if width == 0 || height == 0 || rate <= 0.0 {
            return Err(ToolError::ReadError);
        }

        let temporary = staging_name(input, &format!("speed.{source_ext}"));
        let raw = staging_name(input, "speed-raw.bin");
        let retime = staging_name(input, "speed-frames.bin");
        let sound = staging_name(input, "speed-sound.wav");
        let _sweeps = (
            SweepOf(temporary.clone()),
            SweepOf(raw.clone()),
            SweepOf(retime.clone()),
            SweepOf(sound.clone()),
        );

        let frame_bytes = (width as usize) * (height as usize) * 4;
        let decode_args: Vec<OsString> = [
            "-hide_banner",
            "-nostdin",
            "-y",
            "-i",
        ]
        .iter()
        .map(|word| OsString::from(*word))
        .chain([input.as_os_str().to_os_string()])
        .chain(
            [
                "-pix_fmt",
                "rgba",
                "-f",
                "rawvideo",
                "pipe:1",
            ]
            .iter()
            .map(|word| OsString::from(*word)),
        )
        .collect();
        let raw_file = std::fs::File::create(&raw).map_err(|_| ToolError::WriteError)?;
        let decode = SystemRunner
            .run(&self.ffmpeg_tools.ffmpeg, &decode_args, Some(&raw_file))
            .map_err(|_| ToolError::ToolFailed)?;
        if !decode.succeeded() {
            let _ = std::fs::remove_file(&raw);
            return Err(ToolError::ToolFailed);
        }
        let gathered = std::fs::read(&raw).map_err(|_| ToolError::ReadError)?;
        let _ = std::fs::remove_file(&raw);
        let held = gathered.len() / frame_bytes;
        if held == 0 {
            return Err(ToolError::ReadError);
        }

        let pace = percent as f64 / 100.0;
        let said = ((held as f64) / pace).round().max(1.0) as usize;
        let mut retimed = vec![0u8; said * frame_bytes];
        for moment in 0..said {
            let stand = (((moment as f64) * pace).floor() as usize).min(held - 1);
            retimed[moment * frame_bytes..(moment + 1) * frame_bytes]
                .copy_from_slice(&gathered[stand * frame_bytes..(stand + 1) * frame_bytes]);
        }
        let _ = std::fs::remove_file(&retime);
        std::fs::write(&retime, &retimed).map_err(|_| ToolError::WriteError)?;

        let mut audio_args: Vec<OsString> = Vec::new();
        if let Some(voice) = crate::media::audio::decode_full_pcm(&self.ffmpeg_tools, input) {
            let channels = (voice.channels as usize).max(1);
            let held_notes = voice.samples.len() / channels;
            if held_notes > 0 {
                let said_notes = ((held_notes as f64) / pace).round().max(1.0) as usize;
                let mut heard = Vec::with_capacity(said_notes * channels);
                for moment in 0..said_notes {
                    let stand = (((moment as f64) * pace).floor() as usize).min(held_notes - 1);
                    heard.extend_from_within(stand * channels..(stand + 1) * channels);
                }
                let sung = crate::media::audio::encode_wav_bytes(
                    &heard,
                    voice.sample_rate,
                    voice.channels,
                );
                let _ = std::fs::remove_file(&sound);
                std::fs::write(&sound, &sung).map_err(|_| ToolError::WriteError)?;
                audio_args = [
                    "-i",
                ]
                .iter()
                .map(|word| OsString::from(*word))
                .chain([sound.as_os_str().to_os_string()])
                .chain(
                    [
                        "-c:a",
                        "aac",
                    ]
                    .iter()
                    .map(|word| OsString::from(*word)),
                )
                .collect();
            }
        }

        let mut arguments: Vec<OsString> = [
            "-hide_banner",
            "-nostdin",
            "-y",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgba",
            "-s",
        ]
        .iter()
        .map(|word| OsString::from(*word))
        .chain([OsString::from(format!("{width}x{height}"))])
        .chain(
            [
                "-framerate",
            ]
            .iter()
            .map(|word| OsString::from(*word))
            .chain([OsString::from(format!("{rate:.6}"))]),
        )
        .chain(["-i"].iter().map(|word| OsString::from(*word)))
        .chain([retime.as_os_str().to_os_string()])
        .collect();
        let silent = audio_args.is_empty();
        arguments.extend(audio_args);
        if silent {
            arguments.push(OsString::from("-an"));
        }
        arguments.extend(
            [
                "-c:v",
                "mpeg4",
                "-pix_fmt",
                "yuv420p",
                "-f",
            ]
            .iter()
            .map(|word| OsString::from(*word)),
        );
        arguments.push(OsString::from(source.write.muxer.clone()));
        arguments.push(temporary.as_os_str().to_os_string());
        let made = SystemRunner
            .run(&self.ffmpeg_tools.ffmpeg, &arguments, None)
            .map_err(|_| ToolError::ToolFailed)?;
        let _ = std::fs::remove_file(&retime);
        let _ = std::fs::remove_file(&sound);
        if !made.succeeded() {
            let _ = std::fs::remove_file(&temporary);
            return Err(ToolError::ToolFailed);
        }

        let finals = speed_final_names(input, &speed_saying(percent), &source_ext);
        self.publish(&finals, &temporary)
    }

    /// Joins multiple videos into one MP4 movie, clips scaled to fit the first clip.
    pub fn merge_videos(&self, inputs: &[PathBuf]) -> Result<PathBuf, ToolError> {
        let first = &inputs[0];
        let stem = first
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "video".to_string());

        let (raw_w, raw_h, raw_rate) = crate::media::frames::shape_of(&self.ffmpeg_tools, first)
            .ok_or(ToolError::ReadError)?;
        if raw_w == 0 || raw_h == 0 {
            return Err(ToolError::ReadError);
        }
        let target_w = (raw_w / 2) * 2;
        let target_h = (raw_h / 2) * 2;
        let target_fps = if raw_rate >= 10.0 && raw_rate <= 60.0 {
            raw_rate
        } else {
            30.0
        };

        let temp_video_raw = first.with_file_name(format!("{stem}.merge-video.bin"));
        let temp_audio_wav = first.with_file_name(format!("{stem}.merge-sound.wav"));
        let temp_output = first.with_file_name(format!("{stem}.merge-part.mp4"));

        let _ = std::fs::remove_file(&temp_video_raw);
        let _ = std::fs::remove_file(&temp_audio_wav);
        let _ = std::fs::remove_file(&temp_output);

        let mut raw_video_out = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&temp_video_raw)
            .map_err(|_| ToolError::WriteError)?;

        let mut all_audio_samples: Vec<i16> = Vec::new();

        for input in inputs {
            let (clip_w, clip_h, _) = crate::media::frames::shape_of(&self.ffmpeg_tools, input)
                .ok_or(ToolError::ReadError)?;
            if clip_w == 0 || clip_h == 0 {
                let _ = std::fs::remove_file(&temp_video_raw);
                return Err(ToolError::ReadError);
            }
            let clip_frame_bytes = (clip_w as usize) * (clip_h as usize) * 4;

            let clip_raw_path = input.with_file_name(format!("{stem}.clip-decode.bin"));
            let _ = std::fs::remove_file(&clip_raw_path);
            let clip_file = std::fs::File::create(&clip_raw_path).map_err(|_| ToolError::WriteError)?;

            let decode_args: Vec<OsString> = [
                "-hide_banner",
                "-nostdin",
                "-y",
                "-i",
            ]
            .iter()
            .map(|word| OsString::from(*word))
            .chain([input.as_os_str().to_os_string()])
            .chain(
                [
                    "-pix_fmt",
                    "rgba",
                    "-f",
                    "rawvideo",
                    "pipe:1",
                ]
                .iter()
                .map(|word| OsString::from(*word)),
            )
            .collect();

            let decode = SystemRunner
                .run(&self.ffmpeg_tools.ffmpeg, &decode_args, Some(&clip_file))
                .map_err(|_| ToolError::ToolFailed)?;
            if !decode.succeeded() {
                let _ = std::fs::remove_file(&clip_raw_path);
                let _ = std::fs::remove_file(&temp_video_raw);
                return Err(ToolError::ToolFailed);
            }

            let gathered = std::fs::read(&clip_raw_path).map_err(|_| ToolError::ReadError)?;
            let _ = std::fs::remove_file(&clip_raw_path);
            let clip_frames = gathered.len() / clip_frame_bytes;
            if clip_frames == 0 {
                let _ = std::fs::remove_file(&temp_video_raw);
                return Err(ToolError::ReadError);
            }

            if clip_w == target_w && clip_h == target_h {
                use std::io::Write;
                raw_video_out.write_all(&gathered).map_err(|_| ToolError::WriteError)?;
            } else {
                use std::io::Write;
                let scale_x = target_w as f64 / clip_w as f64;
                let scale_y = target_h as f64 / clip_h as f64;
                let scale = scale_x.min(scale_y);
                let fit_w = ((clip_w as f64 * scale).round() as u32).max(1).min(target_w);
                let fit_h = ((clip_h as f64 * scale).round() as u32).max(1).min(target_h);
                let offset_x = (target_w - fit_w) / 2;
                let offset_y = (target_h - fit_h) / 2;

                for f in 0..clip_frames {
                    let frame_slice = &gathered[f * clip_frame_bytes..(f + 1) * clip_frame_bytes];
                    let src_img = image::RgbaImage::from_raw(clip_w, clip_h, frame_slice.to_vec())
                        .ok_or(ToolError::ReadError)?;
                    let resized = image::imageops::resize(&src_img, fit_w, fit_h, image::imageops::FilterType::Nearest);
                    let mut canvas = image::RgbaImage::from_pixel(target_w, target_h, image::Rgba([0, 0, 0, 255]));
                    image::imageops::overlay(&mut canvas, &resized, offset_x as i64, offset_y as i64);
                    raw_video_out.write_all(canvas.as_raw()).map_err(|_| ToolError::WriteError)?;
                }
            }

            let clip_duration = crate::media::frames::duration_of(&self.ffmpeg_tools, input)
                .unwrap_or(clip_frames as f64 / target_fps);
            let target_audio_frames = (clip_duration * 48000.0).round() as usize;

            if let Some(voice) = crate::media::audio::decode_full_pcm(&self.ffmpeg_tools, input) {
                let channels = (voice.channels as usize).max(1);
                let src_audio_frames = voice.samples.len() / channels;
                if src_audio_frames > 0 {
                    let mut resampled_stereo = Vec::with_capacity(target_audio_frames * 2);
                    for i in 0..target_audio_frames {
                        let src_idx = (((i as f64) * (src_audio_frames as f64) / (target_audio_frames as f64)).floor() as usize).min(src_audio_frames - 1);
                        if channels == 1 {
                            let s = voice.samples[src_idx];
                            resampled_stereo.push(s);
                            resampled_stereo.push(s);
                        } else {
                            let l = voice.samples[src_idx * channels];
                            let r = voice.samples[src_idx * channels + 1];
                            resampled_stereo.push(l);
                            resampled_stereo.push(r);
                        }
                    }
                    all_audio_samples.extend(resampled_stereo);
                } else {
                    all_audio_samples.extend(vec![0i16; target_audio_frames * 2]);
                }
            } else {
                all_audio_samples.extend(vec![0i16; target_audio_frames * 2]);
            }
        }
        drop(raw_video_out);

        let mut encode_args: Vec<OsString> = [
            "-hide_banner",
            "-nostdin",
            "-y",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgba",
            "-s",
        ]
        .iter()
        .map(|w| OsString::from(*w))
        .collect();

        encode_args.push(OsString::from(format!("{target_w}x{target_h}")));
        encode_args.push(OsString::from("-r"));
        encode_args.push(OsString::from(format!("{target_fps}")));
        encode_args.push(OsString::from("-i"));
        encode_args.push(temp_video_raw.as_os_str().to_os_string());

        let has_audio = !all_audio_samples.is_empty();
        if has_audio {
            let wav_bytes = crate::media::audio::encode_wav_bytes(&all_audio_samples, 48000, 2);
            std::fs::write(&temp_audio_wav, &wav_bytes).map_err(|_| ToolError::WriteError)?;
            encode_args.push(OsString::from("-i"));
            encode_args.push(temp_audio_wav.as_os_str().to_os_string());
        }

        encode_args.extend(
            [
                "-c:v",
                "mpeg4",
                "-q:v",
                "5",
            ]
            .iter()
            .map(|w| OsString::from(*w)),
        );

        if has_audio {
            encode_args.extend(
                [
                    "-c:a",
                    "aac",
                    "-b:a",
                    "192k",
                ]
                .iter()
                .map(|w| OsString::from(*w)),
            );
        }

        encode_args.push(OsString::from("-pix_fmt"));
        encode_args.push(OsString::from("yuv420p"));
        encode_args.push(OsString::from("-f"));
        encode_args.push(OsString::from("mov"));
        encode_args.push(temp_output.as_os_str().to_os_string());

        let encode = SystemRunner
            .run(&self.ffmpeg_tools.ffmpeg, &encode_args, None)
            .map_err(|_| ToolError::ToolFailed)?;

        let _ = std::fs::remove_file(&temp_video_raw);
        let _ = std::fs::remove_file(&temp_audio_wav);

        if !encode.succeeded() || !temp_output.is_file() {
            let _ = std::fs::remove_file(&temp_output);
            return Err(ToolError::ToolFailed);
        }

        let dest_names = metadata_final_names(first, "joined", "mp4");
        self.publish(&dest_names, &temp_output)
    }
}

/// The candidate file names for speed output.
pub fn speed_final_names(original: &Path, saying: &str, extension: &str) -> Vec<PathBuf> {
    let stem = original
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".to_string());
    let parent = original.parent().map(|p| p.to_path_buf());
    let mut names = Vec::new();

    let first = format!("{stem} at {saying}.{extension}");
    names.push(match &parent {
        Some(p) => p.join(first),
        None => PathBuf::from(first),
    });

    for index in 2..1000u32 {
        let name = format!("{stem} at {saying} ({index}).{extension}");
        names.push(match &parent {
            Some(p) => p.join(name),
            None => PathBuf::from(name),
        });
    }

    names
}

/// The human-readable speed phrase.
pub fn speed_saying(percent: u32) -> String {
    if percent % 100 == 0 {
        format!("{}x", percent / 100)
    } else if percent % 10 == 0 {
        format!("{:.1}x", percent as f64 / 100.0)
    } else {
        format!("{:.2}x", percent as f64 / 100.0)
    }
}

/// The arguments that trim a media file to start/end milliseconds.
pub fn trim_args(
    input: &Path,
    source: &FormatSupport,
    target: &FormatSupport,
    output_path: Option<&Path>,
    settings: &ToolSettings,
) -> Vec<OsString> {
    let seconds = |milliseconds: u64| format!("{}.{:03}", milliseconds / 1000, milliseconds % 1000);

    let mut args = vec![
        OsString::from("-hide_banner"),
        OsString::from("-nostdin"),
        OsString::from("-y"),
    ];
    if let Some(start) = settings.start_ms.filter(|ms| *ms > 0) {
        args.push(OsString::from("-ss"));
        args.push(OsString::from(seconds(start)));
    }
    args.push(OsString::from("-f"));
    args.push(OsString::from(&source.read.demuxer));
    args.push(OsString::from("-i"));
    args.push(input.as_os_str().to_os_string());
    match (settings.start_ms, settings.end_ms) {
        (Some(start), Some(end)) if end > start => {
            args.push(OsString::from("-t"));
            args.push(OsString::from(seconds(end - start)));
        }
        (None, Some(end)) if end > 0 => {
            args.push(OsString::from("-t"));
            args.push(OsString::from(seconds(end)));
        }
        _ => {}
    }
    args.push(OsString::from("-map"));
    args.push(OsString::from("0"));
    args.push(OsString::from("-c"));
    args.push(OsString::from("copy"));
    args.push(OsString::from("-f"));
    args.push(OsString::from(&target.write.muxer));
    match output_path {
        Some(path) => args.push(path.as_os_str().to_os_string()),
        None => args.push(OsString::from("pipe:1")),
    }
    args
}

/// The folder a split's clips stand in.
pub fn unique_folder(under: &Path, name: &str) -> Result<PathBuf, ToolError> {
    let mut candidate = under.join(name);
    if !candidate.exists() {
        std::fs::create_dir_all(&candidate).map_err(|_| ToolError::WriteError)?;
        return Ok(candidate);
    }
    for number in 2..1000u32 {
        candidate = under.join(format!("{name} {number}"));
        if !candidate.exists() {
            std::fs::create_dir_all(&candidate).map_err(|_| ToolError::WriteError)?;
            return Ok(candidate);
        }
    }
    Err(ToolError::WriteError)
}

/// The stretches the video is cut into, from the points asked for.
pub fn split_segments(points_ms: &[u64], duration: f64) -> Vec<(u64, u64)> {
    let end_ms = (duration * 1000.0).round() as u64;
    let mut bounded: Vec<u64> = points_ms
        .iter()
        .copied()
        .filter(|ms| *ms > 0 && *ms < end_ms)
        .collect();
    bounded.sort_unstable();
    let mut bounds: Vec<u64> = vec![0];
    bounds.extend(bounded);
    bounds.push(end_ms);
    let mut segments = Vec::new();
    for pair in bounds.windows(2) {
        let (start, end) = (pair[0], pair[1]);
        if end - start > 50 {
            segments.push((start, end));
        }
    }
    segments
}

/// The rectangle a crop keeps, if the settings carry a whole one.
pub fn crop_rect(settings: &ToolSettings) -> Option<(u32, u32, u32, u32)> {
    Some((
        settings.crop_x?,
        settings.crop_y?,
        settings.crop_w?,
        settings.crop_h?,
    ))
}

/// A size the video encoders take: both sides even.
pub fn even(size: u32) -> u32 {
    size - (size % 2)
}

/// Kept inside the picture: a crop that runs past an edge is pulled back to it.
pub fn clamp_rect(
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    whole_w: u32,
    whole_h: u32,
) -> (u32, u32, u32, u32) {
    let x = x.min(whole_w.saturating_sub(1));
    let y = y.min(whole_h.saturating_sub(1));
    let w = w.min(whole_w - x).max(1);
    let h = h.min(whole_h - y).max(1);
    (x, y, w, h)
}

/// The arguments that cut a video down to the rectangle it was asked for.
pub fn crop_args(
    input: &Path,
    target: &FormatSupport,
    output_path: Option<&Path>,
    rect: (u32, u32, u32, u32),
    output: (Option<u32>, Option<u32>),
) -> Vec<OsString> {
    let (x, y, w, h) = rect;
    let mut filter = format!("crop={w}:{h}:{x}:{y}", w = even(w), h = even(h));
    match (output.0, output.1) {
        (Some(ow), Some(oh)) => filter.push_str(&format!(",scale={}:{}", even(ow), even(oh))),
        (Some(ow), None) => filter.push_str(&format!(",scale={}:{}", even(ow), -1)),
        (None, Some(oh)) => filter.push_str(&format!(",scale=-1:{}", even(oh))),
        (None, None) => {}
    }
    let mut args = vec![
        OsString::from("-hide_banner"),
        OsString::from("-nostdin"),
        OsString::from("-y"),
        OsString::from("-f"),
        OsString::from(&target.read.demuxer),
        OsString::from("-i"),
        input.as_os_str().to_os_string(),
        OsString::from("-vf"),
        OsString::from(filter.replace('&', ",")),
        OsString::from("-c:v"),
        OsString::from(&target.write.encoder),
        OsString::from("-c:a"),
        OsString::from("copy"),
        OsString::from("-f"),
        OsString::from(&target.write.muxer),
    ];
    match output_path {
        Some(path) => args.push(path.as_os_str().to_os_string()),
        None => args.push(OsString::from("pipe:1")),
    }
    args
}

/// Determines the image format from its extension.
pub fn image_format_of(extension: &str) -> Option<image::ImageFormat> {
    match extension {
        "png" => Some(image::ImageFormat::Png),
        "jpg" | "jpeg" => Some(image::ImageFormat::Jpeg),
        "webp" => Some(image::ImageFormat::WebP),
        "gif" => Some(image::ImageFormat::Gif),
        "bmp" => Some(image::ImageFormat::Bmp),
        "ico" => Some(image::ImageFormat::Ico),
        "tiff" | "tif" => Some(image::ImageFormat::Tiff),
        _ => None,
    }
}

/// Cuts a picture down to the rectangle it was asked for.
pub fn crop_image(
    input: &Path,
    temporary: &Path,
    settings: &ToolSettings,
) -> Result<(), ToolError> {
    let extension = input
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    let Some(format) = image_format_of(&extension) else {
        return Err(ToolError::UnsupportedKind);
    };
    let picture = image::open(input).map_err(|_| ToolError::ReadError)?;
    let Some((x, y, w, h)) = crop_rect(settings) else {
        return Err(ToolError::NothingToDo);
    };
    let (x, y, w, h) = clamp_rect(x, y, w, h, picture.width(), picture.height());
    let mut cut = picture.crop_imm(x, y, w, h);
    if let (Some(ow), Some(oh)) = (settings.width, settings.height) {
        if ow > 0 && oh > 0 {
            cut = cut.resize_exact(ow, oh, image::imageops::FilterType::Lanczos3);
        }
    }
    cut.save_with_format(temporary, format)
        .map_err(|_| ToolError::WriteError)
}
