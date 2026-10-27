use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use crate::diagnostics::{self, Level};
use crate::media::manifest::Manifest;
use crate::media::publish::Publisher;
use crate::media::runner::{ProcessRunner, SystemRunner};
use crate::radial::execution::{Failure, Outcome, Request};

use super::{metadata_final_names, staging_name, SweepOf, ToolError, Tools};

impl<P: Publisher> Tools<P> {
    pub(super) fn run_audio_to_video(&self, request: &Request) -> Outcome {
        let manifest = Manifest::get();
        let mut outputs = Vec::new();
        let mut failed = 0usize;
        let mut first_reason: Option<String> = None;
        let settings = request.settings.visualizer_settings();

        for input in &request.inputs {
            let result = self.visualize_audio_one(input, &manifest, &settings);
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

    pub(super) fn visualize_audio_one(
        &self,
        input: &Path,
        manifest: &Manifest,
        settings: &crate::media::visualizer::VisualizerSettings,
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

        let stem = input
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "audio".to_string());
        let temp_frame_png = input.with_file_name(format!("{stem}.visualizer-frame.png"));
        let temporary = input.with_file_name(format!("{stem}.visualizer-part.mp4"));

        let (w, h) = settings.shape.dimensions();
        let peaks = if settings.style != crate::media::visualizer::VisualizerStyle::StillImage {
            let samples = crate::media::audio::pcm_samples(&self.ffmpeg_tools, input)
                .map(|(s, _)| s)
                .unwrap_or_default();
            crate::media::audio::waveform_peaks(&samples, 180)
        } else {
            Vec::new()
        };

        let frame = crate::media::visualizer::render_visualizer_frame(&peaks, w, h, settings);
        frame.save(&temp_frame_png).map_err(|_| ToolError::WriteError)?;

        let mut encode_args: Vec<std::ffi::OsString> = Vec::new();
        encode_args.push(std::ffi::OsString::from("-y"));
        encode_args.push(std::ffi::OsString::from("-loop"));
        encode_args.push(std::ffi::OsString::from("1"));
        encode_args.push(std::ffi::OsString::from("-framerate"));
        encode_args.push(std::ffi::OsString::from("25"));
        encode_args.push(std::ffi::OsString::from("-f"));
        encode_args.push(std::ffi::OsString::from("png_pipe"));
        encode_args.push(std::ffi::OsString::from("-i"));
        encode_args.push(temp_frame_png.as_os_str().to_os_string());
        encode_args.push(std::ffi::OsString::from("-i"));
        encode_args.push(input.as_os_str().to_os_string());
        encode_args.push(std::ffi::OsString::from("-c:v"));
        encode_args.push(std::ffi::OsString::from("mpeg4"));
        encode_args.push(std::ffi::OsString::from("-q:v"));
        encode_args.push(std::ffi::OsString::from("5"));
        encode_args.push(std::ffi::OsString::from("-c:a"));
        encode_args.push(std::ffi::OsString::from("aac"));
        encode_args.push(std::ffi::OsString::from("-b:a"));
        encode_args.push(std::ffi::OsString::from("192k"));
        encode_args.push(std::ffi::OsString::from("-pix_fmt"));
        encode_args.push(std::ffi::OsString::from("yuv420p"));
        encode_args.push(std::ffi::OsString::from("-shortest"));
        encode_args.push(std::ffi::OsString::from("-f"));
        encode_args.push(std::ffi::OsString::from("mov"));
        encode_args.push(temporary.as_os_str().to_os_string());

        let encode_res = SystemRunner.run(&self.ffmpeg_tools.ffmpeg, &encode_args, None);
        let _ = std::fs::remove_file(&temp_frame_png);

        if encode_res.is_err() || !temporary.exists() {
            let _ = std::fs::remove_file(&temporary);
            return Err(ToolError::ToolFailed);
        }

        let finals = metadata_final_names(input, "visualizer", "mp4");
        self.publish(&finals, &temporary)
    }

    /// Normalizes loudness across all files in the request.
    ///
    /// Reads PCM audio samples, computes peak amplitude and applies linear gain
    /// to meet the target true peak limit, writing a normalized file beside the original.
    pub(super) fn run_normalize(&self, request: &Request, runner: &dyn ProcessRunner) -> Outcome {
        let manifest = Manifest::get();
        let mut outputs = Vec::new();
        let mut failed = 0usize;
        let mut first_reason: Option<String> = None;
        for input in &request.inputs {
            let result = self.normalize_one(input, &manifest, runner, &request.settings);
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

    /// Normalizes loudness on one audio file.
    pub(super) fn normalize_one(
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

        let (normalized_samples, _gain) =
            crate::media::audio::normalize_samples(&pcm.samples, settings.true_peak());

        let temporary = staging_name(input, &format!("{source_ext}.part"));
        let _sweep = SweepOf(temporary.clone());

        if source_ext == "wav" {
            let wav_bytes = crate::media::audio::encode_wav_bytes(
                &normalized_samples,
                pcm.sample_rate,
                pcm.channels,
            );
            fs::write(&temporary, wav_bytes).map_err(|_| ToolError::WriteError)?;
        } else {
            let staging_wav = staging_name(input, "staging_norm.wav");
            let _sweep_wav = SweepOf(staging_wav.clone());
            let wav_bytes = crate::media::audio::encode_wav_bytes(
                &normalized_samples,
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

        let finals = metadata_final_names(input, "normalized", &source_ext);
        self.publish(&finals, &temporary)
    }

    /// Converts audio channels on the files in the request.
    ///
    /// Reads PCM audio samples, converts channels according to the chosen mode
    /// (stereo, mono mixdown, left only, right only, swap) and gains, writing
    /// the converted file beside the original.
    pub(super) fn run_channels(&self, request: &Request, runner: &dyn ProcessRunner) -> Outcome {
        let manifest = Manifest::get();
        let mut outputs = Vec::new();
        let mut failed = 0usize;
        let mut first_reason: Option<String> = None;
        for input in &request.inputs {
            let result = self.channels_one(input, &manifest, runner, &request.settings);
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

    /// Converts audio channels on one audio file.
    pub(super) fn channels_one(
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

        let mode = settings.channel_mode();
        let left_gain = settings.left_gain_factor();
        let right_gain = settings.right_gain_factor();

        let converted = crate::media::audio::convert_audio_channels(
            &pcm,
            mode,
            left_gain,
            right_gain,
        );

        let temporary = staging_name(input, &format!("{source_ext}.part"));
        let _sweep = SweepOf(temporary.clone());

        if source_ext == "wav" {
            let wav_bytes = crate::media::audio::encode_wav_bytes(
                &converted.samples,
                converted.sample_rate,
                converted.channels,
            );
            fs::write(&temporary, wav_bytes).map_err(|_| ToolError::WriteError)?;
        } else {
            let staging_wav = staging_name(input, "staging_chan.wav");
            let _sweep_wav = SweepOf(staging_wav.clone());
            let wav_bytes = crate::media::audio::encode_wav_bytes(
                &converted.samples,
                converted.sample_rate,
                converted.channels,
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

        let suffix = if mode == "mono" { "mono" } else { "channels" };
        let finals = metadata_final_names(input, suffix, &source_ext);
        self.publish(&finals, &temporary)
    }
}
