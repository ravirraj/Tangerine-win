    use super::*;
    use crate::radial::execution::{BleepRange, RedactArea, RedactStyle, ToolSettings};

    #[test]
    fn the_resize_filter_takes_what_was_asked() {
        let both = ToolSettings {
            width: Some(800),
            height: Some(600),
            ..Default::default()
        };
        assert_eq!(resize_filter(&both), "scale=800:600");
        let only_width = ToolSettings {
            width: Some(800),
            height: None,
            ..Default::default()
        };
        assert_eq!(resize_filter(&only_width), "scale=800:-1");
        let only_height = ToolSettings {
            width: None,
            height: Some(600),
            ..Default::default()
        };
        assert_eq!(resize_filter(&only_height), "scale=-1:600");
    }

    #[test]
    fn the_resize_filter_without_a_window_makes_half() {
        assert_eq!(resize_filter(&ToolSettings::default()), "scale=iw/2:ih/2");
    }

    #[test]
    fn the_preset_numbers_are_the_two_the_originals_keep() {
        // Balanced keeps most of the look; strong takes half of it away.
        // The same two answers, said each writer's way.
        assert_eq!(jpeg_quality(false), 72);
        assert_eq!(jpeg_quality(true), 50);
        assert_eq!(webp_quality(false), 72);
        assert_eq!(webp_quality(true), 50);
        assert_eq!(audio_bitrate(false), 128_000);
        assert_eq!(audio_bitrate(true), 64_000);
    }

    #[test]
    fn the_video_bits_come_from_the_weight_and_the_length() {
        // A megabyte over ten seconds carries eight hundred thousand bits
        // a second; balanced keeps six tenths of that.
        let estimated = video_bitrate(1_000_000, 10.0, false, None);
        assert_eq!(estimated, 480_000);
        // Strong keeps a smaller share â€” here, the floor.
        assert_eq!(video_bitrate(1_000_000, 10.0, true, None), 300_000);
        // A target size takes the sound's share first and divides the
        // rest over the seconds.
        let asked = video_bitrate(0, 10.0, false, Some(1_000_000));
        assert_eq!(asked, 1_000_000 * 8 * 94 / 100 / 10 - 128_000);
        // Both floors hold, and a video whose length could not be said
        // takes the floor too.
        assert_eq!(video_bitrate(0, 10.0, false, Some(1)), 150_000);
        assert_eq!(video_bitrate(1_000_000, 0.0, false, None), 300_000);
        assert_eq!(video_bitrate(0, 0.0, false, Some(100_000)), 300_000);
    }

    #[test]
    fn the_sound_bits_for_a_target_stay_between_the_two_walls() {
        assert_eq!(audio_bitrate_for_target(1_000_000, 10.0), 320_000);
        assert_eq!(audio_bitrate_for_target(1_000, 10.0), 32_000);
        assert_eq!(audio_bitrate_for_target(100_000, 10.0), 76_800);
        // A sound whose length could not be said keeps the balanced rate.
        assert_eq!(audio_bitrate_for_target(100_000, 0.0), 128_000);
    }

    #[test]
    fn the_compress_filter_squeezes_only_what_was_asked() {
        // Nothing asked, nothing carried.
        assert_eq!(
            compress_video_filter(1920, 1080, 30.0, &ToolSettings::default()),
            None
        );
        // The strong preset squeezes the long edge to three quarters,
        // evened, on whichever side the long edge stands.
        let strong = ToolSettings {
            compress_strong: true,
            ..Default::default()
        };
        assert_eq!(
            compress_video_filter(1920, 1080, 30.0, &strong).as_deref(),
            Some("scale=1440:-2")
        );
        assert_eq!(
            compress_video_filter(1080, 1920, 30.0, &strong).as_deref(),
            Some("scale=-2:1440")
        );
        // A resolution cap squeezes further when it is the smaller limit,
        // and a video is never made larger than it stood.
        let capped = ToolSettings {
            max_long_edge: Some(720),
            ..Default::default()
        };
        assert_eq!(
            compress_video_filter(1920, 1080, 30.0, &capped).as_deref(),
            Some("scale=720:-2")
        );
        assert_eq!(
            compress_video_filter(640, 360, 30.0, &capped),
            None
        );
        // A frame-rate cap drops the rate only when the video shows more
        // than it was asked to, and it rides behind the scale.
        let fast = ToolSettings {
            max_fps: Some(30),
            compress_strong: true,
            ..Default::default()
        };
        assert_eq!(
            compress_video_filter(1920, 1080, 60.0, &fast).as_deref(),
            Some("scale=1440:-2,fps=30")
        );
        assert_eq!(compress_video_filter(1920, 1080, 30.0, &fast).as_deref(), Some("scale=1440:-2"));
        assert_eq!(compress_video_filter(1920, 1080, 24.0, &fast).as_deref(), Some("scale=1440:-2"));
    }

    #[test]
    fn the_compress_args_say_what_the_program_should_make() {
        let mp4 = Manifest::get().format("mp4").expect("mp4 is in the manifest");
        let strong = ToolSettings {
            compress_strong: true,
            ..Default::default()
        };
        let filter = compress_video_filter(1920, 1080, 60.0, &strong);
        let args = video_compress_args(
            Path::new("one.mp4"),
            mp4,
            Some(Path::new("one.mp4.part")),
            filter,
            480_000,
            128_000,
        );
        assert_eq!(at(&args, "-i"), 5);
        assert_eq!(at(&args, "-vf"), 7);
        assert_eq!(args[8], OsString::from("scale=1440:-2"));
        let picture = at(&args, "-c:v");
        assert_eq!(args[picture + 1], OsString::from("mpeg4"), "the container's own encoder");
        let bits_at = at(&args, "-b:v");
        assert_eq!(args[bits_at + 1], OsString::from("480000"));
        let sound = at(&args, "-c:a");
        assert_eq!(args[sound + 1], OsString::from("aac"), "the sound is written as AAC");
        let tail = args.last().expect("somewhere for the result to go");
        assert_eq!(tail, &OsString::from("one.mp4.part"));
    }

    #[test]
    fn the_sound_args_carry_the_rate_they_were_given() {
        let mp3 = Manifest::get().format("mp3").expect("mp3 is in the manifest");
        let args = audio_compress_args(
            Path::new("song.mp3"),
            mp3,
            None,
            96_000,
        );
        assert_eq!(at(&args, "-c:a"), 7);
        assert_eq!(args[8], OsString::from("libmp3lame"));
        assert_eq!(at(&args, "-b:a"), 9);
        assert_eq!(args[10], OsString::from("96000"));
        assert_eq!(args.last(), Some(&OsString::from("pipe:1")));
    }

    #[test]
    fn the_webp_args_ask_the_programs_encoder_for_a_quality() {
        let webp = Manifest::get().format("webp").expect("webp is in the manifest");
        let args = webp_args(Path::new("one.webp"), webp, None, 72);
        assert_eq!(at(&args, "-c:v"), 7);
        assert_eq!(args[8], OsString::from("libwebp"));
        assert_eq!(at(&args, "-q:v"), 9);
        assert_eq!(args[10], OsString::from("72"));
        // One frame: a webp that carries a still is the one to make
        // lighter, and the encoder stops at the first.
        assert_eq!(at(&args, "-frames:v"), 11);
    }

    /// Where a word sits in the arguments, if it is there.
    fn at(args: &[OsString], word: &str) -> usize {
        args.iter()
            .position(|said| said.to_string_lossy() == word)
            .unwrap_or(usize::MAX)
    }

    #[test]
    fn the_trim_takes_the_range_it_was_given() {
        let mp4 = crate::media::manifest::Manifest::get()
            .format("mp4")
            .expect("mp4 is in the manifest");
        let asked = ToolSettings {
            start_ms: Some(1_000),
            end_ms: Some(3_500),
            ..Default::default()
        };
        let args = trim_args(
            Path::new("one.mp4"),
            mp4,
            mp4,
            Some(Path::new("one.mp4.part")),
            &asked,
        );
        // The seek comes before the input; the length after it.
        assert!(at(&args, "-ss") < at(&args, "-i"));
        assert!(at(&args, "-i") < at(&args, "-t"));
        assert_eq!(
            args[at(&args, "-ss") + 1].to_string_lossy(),
            "1.000",
            "the seek is said in seconds: {args:?}"
        );
        assert_eq!(
            args[at(&args, "-t") + 1].to_string_lossy(),
            "2.500",
            "the length is the end less the beginning: {args:?}"
        );
        assert!(at(&args, "-c") < args.len());
        assert_eq!(args[at(&args, "-c") + 1].to_string_lossy(), "copy");
        // The whole of every stream is carried, sound and all.
        assert!(args.iter().any(|said| said.to_string_lossy() == "-map"));
        assert_eq!(args[at(&args, "-map") + 1].to_string_lossy(), "0");
    }

    #[test]
    fn the_trim_without_a_range_copies_the_whole() {
        let mp4 = crate::media::manifest::Manifest::get()
            .format("mp4")
            .expect("mp4 is in the manifest");
        let args = trim_args(
            Path::new("one.mp4"),
            mp4,
            mp4,
            Some(Path::new("one.mp4.part")),
            &ToolSettings::default(),
        );
        assert_eq!(at(&args, "-ss"), usize::MAX, "no seek is said");
        assert_eq!(at(&args, "-t"), usize::MAX, "no length is said");
        assert_eq!(args[at(&args, "-c") + 1].to_string_lossy(), "copy");
    }

    #[test]
    fn trim_args_for_audio_stream_copies_the_range() {
        let mp3 = crate::media::manifest::Manifest::get()
            .format("mp3")
            .expect("mp3 is in manifest");
        let asked = ToolSettings {
            start_ms: Some(2_000),
            end_ms: Some(5_500),
            ..Default::default()
        };
        let args = trim_args(
            Path::new("track.mp3"),
            mp3,
            mp3,
            Some(Path::new("track.mp3.part")),
            &asked,
        );
        assert!(at(&args, "-ss") < at(&args, "-i"));
        assert!(at(&args, "-i") < at(&args, "-t"));
        assert_eq!(args[at(&args, "-ss") + 1].to_string_lossy(), "2.000");
        assert_eq!(args[at(&args, "-t") + 1].to_string_lossy(), "3.500");
        assert_eq!(args[at(&args, "-c") + 1].to_string_lossy(), "copy");
        assert_eq!(args[at(&args, "-map") + 1].to_string_lossy(), "0");
    }

    #[test]
    fn a_split_cuts_the_video_into_the_stretches_its_points_make() {
        // Three seconds, one point at one: two clips, 0â€“1 and 1â€“3.
        assert_eq!(
            split_segments(&[1_000], 3.0),
            vec![(0, 1_000), (1_000, 3_000)]
        );
        // Equal parts is what the window says: three points at 1 s apart
        // cut four clips of the same length.
        assert_eq!(
            split_segments(&[1_000, 2_000], 3.0),
            vec![(0, 1_000), (1_000, 2_000), (2_000, 3_000)]
        );
        // The points stand on their own: an empty set asks for nothing.
        assert_eq!(split_segments(&[], 3.0), vec![(0, 3_000)]);
    }

    #[test]
    fn a_split_ignores_blades_and_points_beyond_the_video() {
        // A point at a boundary is not a clip; two at the same moment are
        // one; a stretch of 0.05 s or less is not a clip either.
        assert_eq!(split_segments(&[0, 3_000], 3.0), vec![(0, 3_000)]);
        assert_eq!(split_segments(&[1_000, 1_000], 3.0), vec![(0, 1_000), (1_000, 3_000)]);
        // 500â€“550 is 0.05 s exactly: the original skips it, so 550 keeps.
        assert_eq!(
            split_segments(&[500, 550, 2_000], 3.0),
            vec![(0, 500), (550, 2_000), (2_000, 3_000)]
        );
        // Points outside the length are not points: a shorter video cut at
        // the marks of a longer one still cuts what it has.
        assert_eq!(split_segments(&[1_000, 5_000], 3.0), vec![(0, 1_000), (1_000, 3_000)]);
        // and a video whose length nobody knows is not cut at all.
        assert_eq!(split_segments(&[1_000], 0.0), vec![]);
    }

    #[test]
    fn a_clips_folder_that_is_taken_gets_a_number() {
        let under = std::env::temp_dir().join("open-tangerine-unique-folder");
        let _ = std::fs::remove_dir_all(&under);
        std::fs::create_dir_all(&under).expect("the scratch folder was made");
        let first = unique_folder(&under, "claw clips").expect("the folder was made");
        assert_eq!(first.file_name().unwrap().to_string_lossy(), "claw clips");
        assert!(first.is_dir(), "the folder is there as soon as it is named");
        let second = unique_folder(&under, "claw clips").expect("the second folder was made");
        assert_eq!(
            second.file_name().unwrap().to_string_lossy(),
            "claw clips 2",
            "a taken name is numbered"
        );
        assert!(second.is_dir(), "the second folder is made when it is named");
        let _ = std::fs::remove_dir_all(&under);
    }

    #[test]
    #[ignore = "needs the conversion program"]
    fn a_video_is_split_into_a_folder_of_numbered_clips() {
        struct Renamer;
        impl crate::media::publish::Publisher for Renamer {
            fn publish(&self, temporary: &Path, destination: &Path) -> Result<(), crate::media::publish::PublishError> {
                if destination.is_file() {
                    return Err(crate::media::publish::PublishError::DestinationExists);
                }
                std::fs::rename(temporary, destination)
                    .map_err(|_| crate::media::publish::PublishError::CouldNotMove)
            }
        }
        let folder = Path::new("vendor").join("ffmpeg");
        let ffmpeg = folder.join("ffmpeg.exe");
        let ffprobe = folder.join("ffprobe.exe");
        if !ffmpeg.is_file() || !ffprobe.is_file() {
            eprintln!("the program is not there; this test needs it");
            return;
        }
        let tools = crate::media::locator::Tools {
            ffmpeg: ffmpeg.clone(),
            ffprobe: ffprobe.clone(),
        };

        let scratch = std::env::temp_dir().join("open-tangerine-split-test");
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(&scratch).expect("the scratch folder was made");

        // The same three-second video the trim test makes.
        let raw = scratch.join("three.raw");
        let pixels = vec![255u8, 0, 0, 255].repeat(160 * 120 * 30);
        std::fs::write(&raw, pixels).expect("the pixels could be written");
        let input = scratch.join("three.mp4");
        let making: Vec<OsString> = [
            "-hide_banner",
            "-nostdin",
            "-y",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgba",
            "-s",
            "160x120",
            "-r",
            "10",
            "-i",
        ]
        .iter()
        .map(|word| OsString::from(*word))
        .chain([
            raw.as_os_str().to_os_string(),
            OsString::from("-c:v"),
            OsString::from("mpeg4"),
            OsString::from("-pix_fmt"),
            OsString::from("yuv420p"),
            OsString::from("-f"),
            OsString::from("mov"),
            input.as_os_str().to_os_string(),
        ])
        .collect();
        let made = SystemRunner.run(&ffmpeg, &making, None).expect("the video was made");
        assert!(made.succeeded(), "the test video was not made: {}", made.stderr);

        // The split is run the way the wheel runs it: at the one-second
        // mark, which is what the equal-parts slider says for 2 parts.
        let probe = tools.clone();
        let splitter = Tools {
            ffmpeg_tools: tools,
            publisher: Renamer,
        };
        let asked = crate::radial::execution::Request {
            operation: "tools.split".to_string(),
            inputs: vec![input.clone()],
            settings: crate::radial::execution::ToolSettings {
                points_ms: vec![1_000],
                ..Default::default()
            },
        };
        let outcome = splitter.run(&asked, &Selection::inspect(&[]));
        let Outcome::Succeeded { outputs } = outcome else {
            panic!("the split did not succeed");
        };
        // Two clips in the folder beside the original, counted in order,
        // each whole and the length its stretch says.
        assert_eq!(outputs.len(), 2, "one point makes two clips");
        let clips = outputs[0].parent().expect("the clips have a folder");
        assert_eq!(
            clips.file_name().unwrap().to_string_lossy(),
            "three clips",
            "the folder is named for the original"
        );
        assert_eq!(outputs[0].file_name().unwrap().to_string_lossy(), "three part 1.mp4");
        assert_eq!(outputs[1].file_name().unwrap().to_string_lossy(), "three part 2.mp4");
        let first =
            crate::media::frames::duration_of(&probe, &outputs[0]).expect("the first clip has a length");
        assert!(
            (first - 1.0).abs() < 0.35,
            "the first clip is close to a second: {first}"
        );
        let second =
            crate::media::frames::duration_of(&probe, &outputs[1]).expect("the second clip has a length");
        assert!(
            (second - 2.0).abs() < 0.35,
            "the second clip carries the rest: {second}"
        );
        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    #[ignore = "needs the conversion program"]
    fn a_video_is_trimmed_to_the_range_it_was_given() {
        let folder = Path::new("vendor").join("ffmpeg");
        let ffmpeg = folder.join("ffmpeg.exe");
        let ffprobe = folder.join("ffprobe.exe");
        if !ffmpeg.is_file() || !ffprobe.is_file() {
            eprintln!("the program is not there; this test needs it");
            return;
        }
        let tools = crate::media::locator::Tools {
            ffmpeg: ffmpeg.clone(),
            ffprobe: ffprobe.clone(),
        };
        let scratch = std::env::temp_dir().join("open-tangerine-trim-test");
        std::fs::create_dir_all(&scratch).expect("the scratch folder was made");

        // A video of three seconds, ten frames a second, made the way
        // the tests of the media module make their pictures: raw pixels
        // in a file, read by the rawvideo reader.
        let raw = scratch.join("three.raw");
        let pixels = vec![255u8, 0, 0, 255].repeat(160 * 120 * 30);
        std::fs::write(&raw, pixels).expect("the pixels could be written");
        let input = scratch.join("three.mp4");
        let making: Vec<OsString> = [
            "-hide_banner",
            "-nostdin",
            "-y",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgba",
            "-s",
            "160x120",
            "-r",
            "10",
            "-i",
        ]
        .iter()
        .map(|word| OsString::from(*word))
        .chain([
            raw.as_os_str().to_os_string(),
            OsString::from("-c:v"),
            OsString::from("mpeg4"),
            OsString::from("-pix_fmt"),
            OsString::from("yuv420p"),
            OsString::from("-f"),
            OsString::from("mov"),
            input.as_os_str().to_os_string(),
        ])
        .collect();
        let made = SystemRunner.run(&ffmpeg, &making, None).expect("the video was made");
        assert!(made.succeeded(), "the test video was not made: {}", made.stderr);

        // The first second of it is lifted out, and the result says so.
        let mp4 = crate::media::manifest::Manifest::get()
            .format("mp4")
            .expect("mp4 is in the manifest");
        let trimmed = scratch.join("one.mp4");
        let asked = ToolSettings {
            start_ms: Some(0),
            end_ms: Some(1_000),
            ..Default::default()
        };
        let arguments = trim_args(&input, mp4, mp4, Some(&trimmed), &asked);
        let run = SystemRunner
            .run(&ffmpeg, &arguments, None)
            .expect("the trim ran");
        assert!(run.succeeded(), "the trim did not finish: {}", run.stderr);
        let kept = crate::media::frames::duration_of(&tools, &trimmed)
            .expect("the trimmed video has a length");
        assert!(
            (kept - 1.0).abs() < 0.35,
            "the trim kept {kept}s of a 3s video, wanted about 1s"
        );
        // The window's own question: what the video looks like at a
        // moment of it. Halfway into the original there is a frame.
        let frame = crate::media::frames::frame_at(&tools, &input, 1.5)
            .expect("a frame was taken from the middle of the video");
        assert!(frame.width() > 0, "the frame holds a picture");
        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    #[ignore = "needs the conversion program"]
    fn the_moments_asked_for_stand_as_pictures_of_the_own_size() {
        let folder = Path::new("vendor").join("ffmpeg");
        let ffmpeg = folder.join("ffmpeg.exe");
        let ffprobe = folder.join("ffprobe.exe");
        if !ffmpeg.is_file() || !ffprobe.is_file() {
            eprintln!("the program is not there; this test needs it");
            return;
        }
        let scratch = std::env::temp_dir().join("open-tangerine-snapshots-test");
        std::fs::create_dir_all(&scratch).expect("the scratch folder was made");
        let input = make_test_video(&scratch);

        // The snapshots go the whole way through the handler, the way
        // the wheel drives it: the moments are said however the window
        // took them, out of order on purpose, and the pictures stand in
        // the video's own order.
        let tools = Tools {
            ffmpeg_tools: crate::media::locator::Tools {
                ffmpeg,
                ffprobe: ffprobe.clone(),
            },
            publisher: CopyPublisher,
        };
        let request = Request {
            operation: "tools.snapshots".to_string(),
            inputs: vec![input.clone()],
            settings: ToolSettings {
                snapshots_ms: vec![2_500, 0, 1_000],
                ..Default::default()
            },
            ..Default::default()
        };
        let outcome = tools.run(&request, &Selection::inspect(&[input.clone()]));
        let outputs = match outcome {
            Outcome::Succeeded { outputs } => outputs,
            Outcome::Failed { reason } => panic!("the snapshots failed: {:?}", reason),
            Outcome::Cancelled => panic!("the snapshots were not asked to stop"),
        };
        assert_eq!(outputs.len(), 3, "three moments asked, three pictures");
        let shots = outputs[0].parent().expect("the pictures have a folder");
        assert_eq!(
            shots.file_name().unwrap().to_string_lossy(),
            "clip frames",
            "the folder is named for the original"
        );
        assert_eq!(outputs[0].file_name().unwrap().to_string_lossy(), "clip frame 1.png");
        assert_eq!(outputs[1].file_name().unwrap().to_string_lossy(), "clip frame 2.png");
        assert_eq!(outputs[2].file_name().unwrap().to_string_lossy(), "clip frame 3.png");
        // Every picture is a picture: it carries the PNG's own opening
        // words, and it is as wide and as high as the video itself.
        for made in &outputs {
            let bytes = std::fs::read(made).expect("the picture was written");
            assert_eq!(
                &bytes[..8],
                &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a],
                "the picture is a PNG"
            );
            let opened = image::load_from_memory(&bytes).expect("the picture opened");
            assert_eq!((opened.width(), opened.height()), (160, 120));
        }

        // A moment past the video's end is set down, and the taking of
        // the rest still stands. The second taking's folder does not
        // step on the first's.
        let request = Request {
            operation: "tools.snapshots".to_string(),
            inputs: vec![input.clone()],
            settings: ToolSettings {
                snapshots_ms: vec![0, 6_000],
                ..Default::default()
            },
            ..Default::default()
        };
        let outcome = tools.run(&request, &Selection::inspect(&[input.clone()]));
        let outputs = match outcome {
            Outcome::Succeeded { outputs } => outputs,
            Outcome::Failed { reason } => panic!("the later snapshots failed: {:?}", reason),
            Outcome::Cancelled => panic!("the snapshots were not asked to stop"),
        };
        assert_eq!(
            outputs.len(),
            1,
            "one frame of the two asked for was inside the video"
        );
        assert_eq!(
            outputs[0]
                .parent()
                .expect("the pictures have a folder")
                .file_name()
                .unwrap()
                .to_string_lossy(),
            "clip frames 2",
            "the second taking stands beside the first"
        );
        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    fn a_snapshot_run_without_moments_refuses() {
        let tools = Tools {
            ffmpeg_tools: crate::media::locator::Tools {
                ffmpeg: PathBuf::from("nowhere.exe"),
                ffprobe: PathBuf::from("nowhere-probe.exe"),
            },
            publisher: CopyPublisher,
        };
        let request = Request {
            operation: "tools.snapshots".to_string(),
            settings: ToolSettings::default(),
            ..Default::default()
        };
        let outcome = tools.run(&request, &Selection::inspect(&[]));
        let Outcome::Failed { reason } = outcome else {
            panic!("an empty asking should not succeed");
        };
        assert_eq!(
            reason,
            Failure::InvalidInput("there are no snapshots".to_string()),
            "the refusal says what was not asked"
        );
    }

    #[test]
    #[ignore = "needs the conversion program"]
    fn a_video_runs_at_the_speed_it_was_asked_for() {
        let folder = Path::new("vendor").join("ffmpeg");
        let ffmpeg = folder.join("ffmpeg.exe");
        let ffprobe = folder.join("ffprobe.exe");
        if !ffmpeg.is_file() || !ffprobe.is_file() {
            eprintln!("the program is not there; this test needs it");
            return;
        }
        let scratch = std::env::temp_dir().join("open-tangerine-speed-test");
        std::fs::create_dir_all(&scratch).expect("the scratch folder was made");
        let input = make_test_video(&scratch);

        // The copy goes the whole way through the handler, the way the
        // wheel drives it: twice as fast runs half as long, at the same
        // rate of frames, and the sides are what they were.
        let tools = Tools {
            ffmpeg_tools: crate::media::locator::Tools {
                ffmpeg,
                ffprobe: ffprobe.clone(),
            },
            publisher: CopyPublisher,
        };
        let request = Request {
            operation: "tools.speed".to_string(),
            inputs: vec![input.clone()],
            settings: ToolSettings {
                speed_percent: Some(200),
                ..Default::default()
            },
            ..Default::default()
        };
        let outcome = tools.run(&request, &Selection::inspect(&[input.clone()]));
        let outputs = match outcome {
            Outcome::Succeeded { outputs } => outputs,
            Outcome::Failed { reason } => panic!("the fast copy failed: {:?}", reason),
            Outcome::Cancelled => panic!("the run was not asked to stop"),
        };
        assert_eq!(outputs.len(), 1, "one video in, one copy out");
        assert_eq!(
            outputs[0].file_name().unwrap().to_string_lossy(),
            "clip at 2x.mp4",
            "the copy carries the speed it runs at"
        );
        let runs = crate::media::frames::duration_of(&tools.ffmpeg_tools, &outputs[0])
            .expect("the fast copy said its own length");
        assert!(
            (runs - 1.5).abs() < 0.15,
            "the fast copy runs half the time: {runs}"
        );
        let (width, height, rate) =
            crate::media::frames::shape_of(&tools.ffmpeg_tools, &outputs[0])
                .expect("the fast copy said its own shape");
        assert_eq!((width, height), (160, 120), "the sides stand as they were");
        assert!(
            (rate - 10.0).abs() < 0.5,
            "the frames stand at the video's own rate"
        );

        // Half speed stands the other way: the copy runs twice as long.
        let request = Request {
            operation: "tools.speed".to_string(),
            inputs: vec![input.clone()],
            settings: ToolSettings {
                speed_percent: Some(50),
                ..Default::default()
            },
            ..Default::default()
        };
        let outcome = tools.run(&request, &Selection::inspect(&[input.clone()]));
        let outputs = match outcome {
            Outcome::Succeeded { outputs } => outputs,
            Outcome::Failed { reason } => panic!("the slow copy failed: {:?}", reason),
            Outcome::Cancelled => panic!("the run was not asked to stop"),
        };
        assert_eq!(
            outputs[0].file_name().unwrap().to_string_lossy(),
            "clip at 0.5x.mp4",
            "the fraction is said with a point"
        );
        let runs = crate::media::frames::duration_of(&tools.ffmpeg_tools, &outputs[0])
            .expect("the slow copy said its own length");
        assert!(
            (runs - 6.0).abs() < 0.15,
            "the slow copy runs twice the time: {runs}"
        );
        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    fn a_speed_without_a_speed_refuses() {
        let tools = Tools {
            ffmpeg_tools: crate::media::locator::Tools {
                ffmpeg: PathBuf::from("nowhere.exe"),
                ffprobe: PathBuf::from("nowhere-probe.exe"),
            },
            publisher: CopyPublisher,
        };
        let request = Request {
            operation: "tools.speed".to_string(),
            settings: ToolSettings::default(),
            ..Default::default()
        };
        let outcome = tools.run(&request, &Selection::inspect(&[]));
        let Outcome::Failed { reason } = outcome else {
            panic!("an asking without a speed should not succeed");
        };
        assert_eq!(
            reason,
            Failure::InvalidInput("there is no speed to run at".to_string()),
            "the refusal says what was not said"
        );
    }

    #[test]
    #[ignore = "needs the conversion program to run a real audio trim"]
    fn real_audio_trim_with_the_program() {
        let Some(tools) = crate::media::locator::find() else {
            return;
        };
        let ffmpeg = &tools.ffmpeg;
        let scratch = std::env::temp_dir().join(format!("ot-audio-trim-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&scratch);

        // Generate a 3-second test WAV file
        let input = scratch.join("three.wav");
        let pcm = make_wav_bytes(3.0, 44100);
        std::fs::write(&input, pcm).expect("three.wav written");

        let wav = crate::media::manifest::Manifest::get()
            .format("wav")
            .expect("wav is in the manifest");
        let trimmed = scratch.join("one.wav");
        let asked = ToolSettings {
            start_ms: Some(1_000),
            end_ms: Some(2_500),
            ..Default::default()
        };
        let arguments = trim_args(&input, wav, wav, Some(&trimmed), &asked);
        let run = SystemRunner
            .run(ffmpeg, &arguments, None)
            .expect("the audio trim ran");
        assert!(run.succeeded(), "the audio trim did not finish: {}", run.stderr);
        let kept = crate::media::frames::duration_of(&tools, &trimmed)
            .expect("the trimmed audio has a length");
        assert!(
            (kept - 1.5).abs() < 0.25,
            "the audio trim kept {kept}s of audio, wanted about 1.5s"
        );
        let _ = std::fs::remove_dir_all(&scratch);
    }

    struct Nothing;

    impl Publisher for Nothing {
        fn publish(&self, _temporary: &Path, _destination: &Path) -> Result<(), crate::media::publish::PublishError> {
            Ok(())
        }
    }

    #[test]
    fn unknown_operation_is_rejected() {
        let tools = Tools {
            ffmpeg_tools: crate::media::locator::Tools {
                ffmpeg: PathBuf::from("ffmpeg"),
                ffprobe: PathBuf::from("ffprobe"),
            },
            publisher: Nothing,
        };
        let environment = crate::radial::registry::Environment::none();
        let request = Request {
            operation: "tools.nonexistent".to_string(),
            inputs: Vec::new(),
            ..Default::default()
        };
        let outcome = crate::radial::execution::run(&request, &Selection::inspect(&[]), &environment, &tools);
        match outcome {
            Outcome::Failed { reason } => {
                assert!(matches!(reason, Failure::UnknownOperation));
            }
            other => panic!("unexpected outcome: {other:?}"),
        }
    }

    #[test]
    fn the_crop_is_said_the_way_the_program_reads_it() {
        let mp4 = crate::media::manifest::Manifest::get()
            .format("mp4")
            .expect("mp4 is in the manifest");
        let args = crop_args(
            Path::new("one.mp4"),
            mp4,
            Some(Path::new("one.mp4.part")),
            (10, 8, 81, 61),
            (None, None),
        );
        let filter = args
            .iter()
            .position(|said| said.to_string_lossy() == "-vf")
            .map(|at| args[at + 1].to_string_lossy().into_owned())
            .expect("the crop carries a filter");
        // The odd sides are brought down to even ones, and the rectangle
        // is said width, height, then where it starts.
        assert_eq!(filter, "crop=80:60:10:8", "{args:?}");
        // The sound is carried over as it is, not made again.
        let audio = args
            .iter()
            .position(|said| said.to_string_lossy() == "-c:a")
            .map(|at| args[at + 1].to_string_lossy().into_owned())
            .expect("the crop says what its sound does");
        assert_eq!(audio, "copy");
    }

    #[test]
    fn a_crop_that_is_also_resized_says_so() {
        let mp4 = crate::media::manifest::Manifest::get()
            .format("mp4")
            .expect("mp4 is in the manifest");
        let args = crop_args(
            Path::new("one.mp4"),
            mp4,
            Some(Path::new("one.mp4.part")),
            (0, 0, 100, 100),
            (Some(41), None),
        );
        let filter = args
            .iter()
            .position(|said| said.to_string_lossy() == "-vf")
            .map(|at| args[at + 1].to_string_lossy().into_owned())
            .expect("the crop carries a filter");
        assert_eq!(filter, "crop=100:100:0:0,scale=40:-1", "{args:?}");
    }

    #[test]
    fn a_crop_that_runs_past_the_edge_is_pulled_back() {
        // A rect that starts past the picture is brought to its edge.
        assert_eq!(clamp_rect(60, 60, 20, 20, 64, 64), (60, 60, 4, 4));
        // One that is bigger than the picture is cut down to it.
        assert_eq!(clamp_rect(0, 0, 100, 100, 64, 64), (0, 0, 64, 64));
        // And one inside stays exactly as it was.
        assert_eq!(clamp_rect(4, 4, 32, 24, 64, 64), (4, 4, 32, 24));
    }

    /// A publisher shaped like the real one: the move happens only when
    /// nothing stands at the name it is asked for.
    struct RealShaped;

    impl Publisher for RealShaped {
        fn publish(
            &self,
            temporary: &Path,
            destination: &Path,
        ) -> Result<(), crate::media::publish::PublishError> {
            if destination.is_file() {
                return Err(crate::media::publish::PublishError::DestinationExists);
            }
            std::fs::copy(temporary, destination)
                .map(|_| ())
                .map_err(|_| crate::media::publish::PublishError::CouldNotMove)
        }
    }

    #[test]
    fn a_result_that_keeps_the_name_lands_beside_the_original() {
        // A cut that keeps the format wants the original's own name, and
        // the original is standing there: the result takes the next free
        // name instead. Nothing is ever written over.
        let scratch = std::env::temp_dir().join("open-tangerine-publish-beside-test");
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(&scratch).expect("the scratch folder was made");
        let input = scratch.join("one.mp4");
        std::fs::write(&input, b"original").expect("the original could be made");
        let temporary = scratch.join("one.mp4.part");
        std::fs::write(&temporary, b"made").expect("the work could be made");

        let tools = Tools {
            ffmpeg_tools: crate::media::locator::Tools {
                ffmpeg: PathBuf::from("ffmpeg"),
                ffprobe: PathBuf::from("ffprobe"),
            },
            publisher: RealShaped,
        };
        let made = tools
            .publish(&final_names(&input, "mp4"), &temporary)
            .expect("a free name was found");
        assert_eq!(made, scratch.join("one (2).mp4"));
        assert_eq!(
            std::fs::read(&made).expect("the result is there"),
            b"made",
            "what was made is what arrived"
        );
        let _ = std::fs::remove_dir_all(&scratch);
    }

    /// A publisher shaped like the real one: the work is copied to the
    /// name it is given, or refused when something is already standing
    /// there â€” the handler is the one that then tries the next name.
    struct CopyPublisher;

    impl Publisher for CopyPublisher {
        fn publish(
            &self,
            temporary: &Path,
            destination: &Path,
        ) -> Result<(), crate::media::publish::PublishError> {
            if destination.is_file() {
                return Err(crate::media::publish::PublishError::DestinationExists);
            }
            std::fs::copy(temporary, destination)
                .map(|_| ())
                .map_err(|_| crate::media::publish::PublishError::CouldNotMove)
        }
    }

    /// Makes the three-second test video the trim test makes, and says
    /// where it is.
    fn make_test_video(scratch: &Path) -> PathBuf {
        let ffmpeg = Path::new("vendor").join("ffmpeg").join("ffmpeg.exe");
        assert!(ffmpeg.is_file(), "the conversion program is not there");
        let raw = scratch.join("three.raw");
        std::fs::write(&raw, vec![255u8, 0, 0, 255].repeat(160 * 120 * 30))
            .expect("the pixels could be written");
        let input = scratch.join("clip.mp4");
        let making: Vec<OsString> = [
            "-hide_banner",
            "-nostdin",
            "-y",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgba",
            "-s",
            "160x120",
            "-r",
            "10",
            "-i",
        ]
        .iter()
        .map(|word| OsString::from(*word))
        .chain([
            raw.as_os_str().to_os_string(),
            OsString::from("-c:v"),
            OsString::from("mpeg4"),
            OsString::from("-pix_fmt"),
            OsString::from("yuv420p"),
            OsString::from("-f"),
            OsString::from("mov"),
            input.as_os_str().to_os_string(),
        ])
        .collect();
        let made = SystemRunner
            .run(&ffmpeg, &making, None)
            .expect("the video was made");
        assert!(made.succeeded(), "the test video was not made: {}", made.stderr);
        input
    }

    /// How big the picture in a video is, as the probe says.
    fn video_size(ffprobe: &Path, video: &Path) -> (u32, u32) {
        let asking: Vec<OsString> = [
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=width,height",
            "-of",
            "csv=p=0:s=x",
        ]
        .iter()
        .map(|word| OsString::from(*word))
        .chain([video.as_os_str().to_os_string()])
        .collect();
        let run = SystemRunner
            .run(ffprobe, &asking, None)
            .expect("the probe ran");
        assert!(run.succeeded(), "the probe did not finish: {}", run.stderr);
        let said = String::from_utf8_lossy(&run.stdout);
        let (w, h) = said.trim().split_once('x').expect("the probe said a size");
        (w.parse().expect("a width"), h.parse().expect("a height"))
    }

    #[test]
    #[ignore = "needs the conversion program"]
    fn a_video_is_cropped_to_the_rectangle_it_was_asked_for() {
        let folder = Path::new("vendor").join("ffmpeg");
        let ffmpeg = folder.join("ffmpeg.exe");
        let ffprobe = folder.join("ffprobe.exe");
        if !ffmpeg.is_file() || !ffprobe.is_file() {
            eprintln!("the program is not there; this test needs it");
            return;
        }
        let scratch = std::env::temp_dir().join("open-tangerine-crop-video-test");
        std::fs::create_dir_all(&scratch).expect("the scratch folder was made");
        let input = make_test_video(&scratch);

        // The crop goes the whole way through the handler, the way the
        // wheel drives it: the rectangle is odd on purpose, and the even
        // sides are what come out.
        let tools = Tools {
            ffmpeg_tools: crate::media::locator::Tools {
                ffmpeg,
                ffprobe: ffprobe.clone(),
            },
            publisher: CopyPublisher,
        };
        let request = Request {
            operation: "tools.crop".to_string(),
            inputs: vec![input.clone()],
            settings: ToolSettings {
                crop_x: Some(10),
                crop_y: Some(8),
                crop_w: Some(81),
                crop_h: Some(61),
                ..Default::default()
            },
            ..Default::default()
        };
        let outcome = tools.run(&request, &Selection::inspect(&[input.clone()]));
        let outputs = match outcome {
            Outcome::Succeeded { outputs } => outputs,
            Outcome::Failed { reason } => panic!("the crop failed: {:?}", reason),
            Outcome::Cancelled => panic!("the crop was not asked to stop"),
        };
        assert_eq!(outputs.len(), 1, "one file in, one file out");
        let made = outputs.first().expect("the crop is there");
        assert!(made.is_file(), "the cropped video exists");
        let (w, h) = video_size(&ffprobe, made);
        assert_eq!((w, h), (80, 60), "the crop is the rectangle, evened");
        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    #[ignore = "needs the picture library and nothing more"]
    fn a_picture_is_cropped_with_the_programs_own_hands() {
        let scratch = std::env::temp_dir().join("open-tangerine-crop-image-test");
        std::fs::create_dir_all(&scratch).expect("the scratch folder was made");
        let input = scratch.join("photo.png");
        image::DynamicImage::new_rgb8(64, 48)
            .save(&input)
            .expect("the picture was made");

        let tools = Tools {
            ffmpeg_tools: crate::media::locator::Tools {
                ffmpeg: PathBuf::from("ffmpeg"),
                ffprobe: PathBuf::from("ffprobe"),
            },
            publisher: CopyPublisher,
        };
        let request = Request {
            operation: "tools.crop".to_string(),
            inputs: vec![input.clone()],
            settings: ToolSettings {
                crop_x: Some(4),
                crop_y: Some(4),
                crop_w: Some(32),
                crop_h: Some(24),
                ..Default::default()
            },
            ..Default::default()
        };
        let outcome = tools.run(&request, &Selection::inspect(&[input.clone()]));
        let outputs = match outcome {
            Outcome::Succeeded { outputs } => outputs,
            Outcome::Failed { reason } => panic!("the crop failed: {:?}", reason),
            Outcome::Cancelled => panic!("the crop was not asked to stop"),
        };
        let made = outputs.first().expect("the crop is there");
        let picture = image::open(made).expect("the crop reads back");
        assert_eq!((picture.width(), picture.height()), (32, 24));
        let _ = std::fs::remove_dir_all(&scratch);
    }

    // The aliases are the point of these two: a file the wheel calls a
    // picture has to be worked on because of what it is called â€” .jpg is
    // the same file .jpeg is, and a tool that refuses it is a tool that
    // never runs.

    #[test]
    #[ignore = "needs the picture library and nothing more"]
    fn a_jpg_is_cropped_with_the_programs_own_hands() {
        let scratch = std::env::temp_dir().join("open-tangerine-crop-jpg-test");
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(&scratch).expect("the scratch folder was made");
        let input = make_noisy_jpeg(&scratch, "photo.jpg", 90);

        let tools = Tools {
            ffmpeg_tools: crate::media::locator::Tools {
                ffmpeg: PathBuf::from("ffmpeg"),
                ffprobe: PathBuf::from("ffprobe"),
            },
            publisher: CopyPublisher,
        };
        let request = Request {
            operation: "tools.crop".to_string(),
            inputs: vec![input.clone()],
            settings: ToolSettings {
                crop_x: Some(80),
                crop_y: Some(60),
                crop_w: Some(160),
                crop_h: Some(120),
                ..Default::default()
            },
            ..Default::default()
        };
        let outcome = tools.run(&request, &Selection::inspect(&[input.clone()]));
        let outputs = match outcome {
            Outcome::Succeeded { outputs } => outputs,
            Outcome::Failed { reason } => panic!("the jpg crop failed: {:?}", reason),
            Outcome::Cancelled => panic!("the crop was not asked to stop"),
        };
        let made = outputs.first().expect("the crop is there");
        let picture = image::open(made).expect("the crop reads back");
        assert_eq!(
            (picture.width(), picture.height()),
            (160, 120),
            "the quarter of the picture is standing"
        );
        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    #[ignore = "needs the conversion program"]
    fn a_jpg_is_turned_a_quarter_at_the_wheel_path() {
        let folder = Path::new("vendor").join("ffmpeg");
        let ffmpeg = folder.join("ffmpeg.exe");
        let ffprobe = folder.join("ffprobe.exe");
        if !ffmpeg.is_file() || !ffprobe.is_file() {
            eprintln!("the program is not there; this test needs it");
            return;
        }
        let scratch = std::env::temp_dir().join("open-tangerine-rotate-jpg-test");
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(&scratch).expect("the scratch folder was made");
        let input = make_noisy_jpeg(&scratch, "photo.jpg", 90);

        let tools = Tools {
            ffmpeg_tools: crate::media::locator::Tools {
                ffmpeg: ffmpeg.clone(),
                ffprobe: ffprobe.clone(),
            },
            publisher: CopyPublisher,
        };
        let request = Request {
            operation: "tools.rotate".to_string(),
            inputs: vec![input.clone()],
            ..Default::default()
        };
        let outcome = tools.run(&request, &Selection::inspect(&[input.clone()]));
        let outputs = match outcome {
            Outcome::Succeeded { outputs } => outputs,
            Outcome::Failed { reason } => panic!("the jpg rotate failed: {:?}", reason),
            Outcome::Cancelled => panic!("the turn was not asked to stop"),
        };
        let made = outputs.first().expect("the turned copy is there");
        let picture = image::open(made).expect("the copy reads back");
        assert_eq!(
            (picture.width(), picture.height()),
            (240, 320),
            "the turn stood the sides the other way"
        );
        let _ = std::fs::remove_dir_all(&scratch);
    }

    // A photograph saves under its own words: the best of what the
    // window asks is carried through the EXIF block, the way the sound
    // and movie tags are carried through the conversion program's own
    // hands.

    fn request_for(operation: &str, inputs: &[PathBuf], settings: ToolSettings) -> Request {
        Request {
            operation: operation.to_string(),
            inputs: inputs.to_vec(),
            settings,
            ..Default::default()
        }
    }

    #[test]
    fn a_jpg_is_saved_with_its_words() {
        let scratch = std::env::temp_dir().join("open-tangerine-metadata-jpg-test");
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(&scratch).expect("the scratch folder was made");
        let input =
            image::DynamicImage::new_rgb8(64, 48).save(&scratch.join("photo.jpg")).map(|_| scratch.join("photo.jpg")).expect("the picture was made");

        let tools = Tools {
            ffmpeg_tools: crate::media::locator::Tools {
                ffmpeg: PathBuf::from("ffmpeg"),
                ffprobe: PathBuf::from("ffprobe"),
            },
            publisher: CopyPublisher,
        };
        let request = request_for(
            "tools.metadata",
            std::slice::from_ref(&input),
            ToolSettings {
                metadata_tags: vec![
                    ("title".to_string(), "At the harbor".to_string()),
                    ("artist".to_string(), "Ravi".to_string()),
                ],
                ..Default::default()
            },
        );
        let outcome = tools.run(&request, &Selection::inspect(&[input.clone()]));
        let outputs = match outcome {
            Outcome::Succeeded { outputs } => outputs,
            Outcome::Failed { reason } => panic!("the save failed: {:?}", reason),
            Outcome::Cancelled => panic!("the save was not asked to stop"),
        };
        let made = outputs.first().expect("the copy is there");
        let said = crate::media::metadata::read_tags(made, None);
        assert_eq!(
            said.get(&crate::media::metadata::MetadataTag::Title).map(String::as_str),
            Some("At the harbor"),
            "the title the window asked stands in the copy"
        );
        assert_eq!(
            said.get(&crate::media::metadata::MetadataTag::Artist).map(String::as_str),
            Some("Ravi")
        );
        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    fn a_jpg_asked_plain_carries_no_words() {
        let scratch = std::env::temp_dir().join("open-tangerine-metadata-clean-jpg-test");
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(&scratch).expect("the scratch folder was made");
        let input = scratch.join("photo.jpg");
        image::DynamicImage::new_rgb8(64, 48)
            .save(&input)
            .expect("the picture was made");

        let tools = Tools {
            ffmpeg_tools: crate::media::locator::Tools {
                ffmpeg: PathBuf::from("ffmpeg"),
                ffprobe: PathBuf::from("ffprobe"),
            },
            publisher: CopyPublisher,
        };
        let request = request_for(
            "tools.metadata",
            std::slice::from_ref(&input),
            ToolSettings {
                metadata_remove_all: true,
                ..Default::default()
            },
        );
        let outcome = tools.run(&request, &Selection::inspect(&[input.clone()]));
        let outputs = match outcome {
            Outcome::Succeeded { outputs } => outputs,
            Outcome::Failed { reason } => panic!("the clean save failed: {:?}", reason),
            Outcome::Cancelled => panic!("the clean save was not asked to stop"),
        };
        let made = outputs.first().expect("the clean copy is there");
        let picture = image::open(made).expect("the picture reads back");
        assert_eq!(
            (picture.width(), picture.height()),
            (64, 48),
            "the picture is the same picture, with nothing said over it"
        );
        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    fn an_area_lands_on_the_pixels_of_its_rectangle() {
        let area = RedactArea {
            x_mille: 250,
            y_mille: 500,
            w_mille: 500,
            h_mille: 500,
            start_ms: None,
            end_ms: None,
        };
        assert_eq!(area.rect_on(80, 60), (20, 30, 40, 30));
        // what hangs off the right edge is cut away
        let wide = RedactArea {
            x_mille: 900,
            y_mille: 500,
            w_mille: 500,
            h_mille: 500,
            start_ms: None,
            end_ms: None,
        };
        assert_eq!(wide.rect_on(80, 60), (72, 30, 8, 30));
        // at the very edge there is nothing left of the rectangle
        let edge = RedactArea { x_mille: 1000, ..wide };
        assert_eq!(edge.rect_on(80, 60), (80, 30, 0, 30));
    }

    #[test]
    fn the_redact_look_answers_the_reference_defaults() {
        let settings = ToolSettings::default();
        assert_eq!(settings.redaction_style(), RedactStyle::Solid);
        assert_eq!(settings.redaction_strength_fraction(), 0.5);
        assert_eq!(settings.redaction_color(), [0, 0, 0]);
        // the reference's own rule: the strength takes the look between
        // 2% and 30% of the smaller side
        let quiet = ToolSettings {
            redact_strength: Some(0),
            ..ToolSettings::default()
        };
        let loud = ToolSettings {
            redact_strength: Some(100),
            ..ToolSettings::default()
        };
        assert!((quiet.redaction_effect(1_000) - 20.0).abs() < 1e-9);
        assert!((loud.redaction_effect(1_000) - 300.0).abs() < 1e-9);
        // the hex is read as three bytes
        let colored = ToolSettings {
            redact_color: Some("ff5a1f".to_string()),
            ..ToolSettings::default()
        };
        assert_eq!(colored.redaction_color(), [255, 90, 31]);
    }

    #[test]
    fn a_redaction_without_areas_is_refused_and_said_so() {
        let tools = Tools {
            ffmpeg_tools: crate::media::locator::Tools {
                ffmpeg: PathBuf::from("ffmpeg"),
                ffprobe: PathBuf::from("ffprobe"),
            },
            publisher: CopyPublisher,
        };
        let request = Request {
            operation: "tools.redact_image".to_string(),
            inputs: vec![PathBuf::from("photo.png")],
            settings: ToolSettings::default(),
            ..Default::default()
        };
        let outcome = tools.run(&request, &Selection::inspect(&[PathBuf::from("photo.png")]));
        match outcome {
            Outcome::Failed {
                reason: Failure::InvalidInput(message),
            } => {
                assert_eq!(message, "there are no redaction areas");
            }
            other => panic!("the redaction was not refused for saying nothing: {other:?}"),
        }
    }

    #[test]
    #[ignore = "needs the picture library and nothing more"]
    fn a_picture_is_redacted_with_the_programs_own_hands() {
        let scratch = std::env::temp_dir().join("open-tangerine-redact-image-test");
        std::fs::create_dir_all(&scratch).expect("the scratch folder was made");
        let input = scratch.join("photo.png");
        let mut picture = image::DynamicImage::new_rgb8(64, 48);
        for (_, _, tone) in picture.as_mut_rgb8().expect("rgb8").enumerate_pixels_mut() {
            *tone = image::Rgb([255, 255, 255]);
        }
        picture.save(&input).expect("the picture was made");

        let tools = Tools {
            ffmpeg_tools: crate::media::locator::Tools {
                ffmpeg: PathBuf::from("ffmpeg"),
                ffprobe: PathBuf::from("ffprobe"),
            },
            publisher: CopyPublisher,
        };
        let request = Request {
            operation: "tools.redact_image".to_string(),
            inputs: vec![input.clone()],
            settings: ToolSettings {
                redact_areas: vec![RedactArea {
                    x_mille: 250,
                    y_mille: 250,
                    w_mille: 500,
                    h_mille: 500,
                    start_ms: None,
                    end_ms: None,
                }],
                ..Default::default()
            },
            ..Default::default()
        };
        let outcome = tools.run(&request, &Selection::inspect(&[input.clone()]));
        let outputs = match outcome {
            Outcome::Succeeded { outputs } => outputs,
            Outcome::Failed { reason } => panic!("the redaction failed: {reason:?}"),
            Outcome::Cancelled => panic!("the redaction was not asked to stop"),
        };
        let made = outputs.first().expect("the redaction is there");
        assert!(made.is_file(), "the covered copy exists");
        assert_eq!(
            made.file_name().and_then(|n| n.to_str()),
            Some("photo redacted.png")
        );
        let read = image::open(made).expect("the copy reads back").to_rgba8();
        assert_eq!(
            read.get_pixel(32, 24),
            &image::Rgba([0, 0, 0, 255]),
            "the middle of the box wears the color it was asked"
        );
        assert_eq!(
            read.get_pixel(2, 2),
            &image::Rgba([255, 255, 255, 255]),
            "what was not covered is as it was"
        );
        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    #[ignore = "needs the picture library and nothing more"]
    fn a_blurred_redaction_smears_what_it_covers_and_nothing_else() {
        let scratch = std::env::temp_dir().join("open-tangerine-redact-blur-test");
        std::fs::create_dir_all(&scratch).expect("the scratch folder was made");
        let input = scratch.join("photo.png");
        let mut picture = image::DynamicImage::new_rgb8(64, 48);
        for (_, _, tone) in picture.as_mut_rgb8().expect("rgb8").enumerate_pixels_mut() {
            *tone = image::Rgb([255, 255, 255]);
        }
        // a black square of the picture's own middle: the thing to smear
        for y in 18..30 {
            for x in 24..40 {
                picture.as_mut_rgb8().expect("rgb8").put_pixel(
                    x,
                    y,
                    image::Rgb([0, 0, 0]),
                );
            }
        }
        picture.save(&input).expect("the picture was made");

        let tools = Tools {
            ffmpeg_tools: crate::media::locator::Tools {
                ffmpeg: PathBuf::from("ffmpeg"),
                ffprobe: PathBuf::from("ffprobe"),
            },
            publisher: CopyPublisher,
        };
        let request = Request {
            operation: "tools.redact_image".to_string(),
            inputs: vec![input.clone()],
            settings: ToolSettings {
                redact_style: Some(RedactStyle::Blur),
                redact_areas: vec![RedactArea {
                    x_mille: 250,
                    y_mille: 250,
                    w_mille: 500,
                    h_mille: 500,
                    start_ms: None,
                    end_ms: None,
                }],
                ..Default::default()
            },
            ..Default::default()
        };
        let outcome = tools.run(&request, &Selection::inspect(&[input.clone()]));
        let outputs = match outcome {
            Outcome::Succeeded { outputs } => outputs,
            Outcome::Failed { reason } => panic!("the redaction failed: {reason:?}"),
            Outcome::Cancelled => panic!("the redaction was not asked to stop"),
        };
        let made = outputs.first().expect("the redaction is there");
        let read = image::open(made).expect("the copy reads back").to_rgba8();
        let middle = read.get_pixel(32, 24).0;
        assert_ne!(middle[0], 0, "the smeared middle is not the black it was");
        assert_ne!(middle[0], 255, "the smeared middle is not white either");
        assert_eq!(
            read.get_pixel(2, 2),
            &image::Rgba([255, 255, 255, 255]),
            "far outside the area nothing changed"
        );
        assert_eq!(
            read.get_pixel(10, 40),
            &image::Rgba([255, 255, 255, 255]),
            "just outside the area nothing changed"
        );
        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    #[ignore = "needs the picture library and nothing more"]
    fn a_pixelated_redaction_blocks_what_it_covers_and_nothing_else() {
        let scratch = std::env::temp_dir().join("open-tangerine-redact-block-test");
        std::fs::create_dir_all(&scratch).expect("the scratch folder was made");
        let input = scratch.join("photo.png");
        let mut picture = image::DynamicImage::new_rgb8(64, 48);
        for (_, _, tone) in picture.as_mut_rgb8().expect("rgb8").enumerate_pixels_mut() {
            *tone = image::Rgb([255, 255, 255]);
        }
        // a black square inside the area, its edge crossing a block: the
        // block that straddles it comes out mixed, the middle keeps the
        // secret's dark
        for y in 18..30 {
            for x in 24..40 {
                picture.as_mut_rgb8().expect("rgb8").put_pixel(
                    x,
                    y,
                    image::Rgb([0, 0, 0]),
                );
            }
        }
        picture.save(&input).expect("the picture was made");

        let tools = Tools {
            ffmpeg_tools: crate::media::locator::Tools {
                ffmpeg: PathBuf::from("ffmpeg"),
                ffprobe: PathBuf::from("ffprobe"),
            },
            publisher: CopyPublisher,
        };
        let request = Request {
            operation: "tools.redact_image".to_string(),
            inputs: vec![input.clone()],
            settings: ToolSettings {
                redact_style: Some(RedactStyle::Pixelate),
                redact_areas: vec![RedactArea {
                    x_mille: 250,
                    y_mille: 250,
                    w_mille: 500,
                    h_mille: 500,
                    start_ms: None,
                    end_ms: None,
                }],
                ..Default::default()
            },
            ..Default::default()
        };
        let outcome = tools.run(&request, &Selection::inspect(&[input.clone()]));
        let outputs = match outcome {
            Outcome::Succeeded { outputs } => outputs,
            Outcome::Failed { reason } => panic!("the redaction failed: {reason:?}"),
            Outcome::Cancelled => panic!("the redaction was not asked to stop"),
        };
        let made = outputs.first().expect("the redaction is there");
        let read = image::open(made).expect("the copy reads back").to_rgba8();
        // the middle sits inside a block that is still the secret's
        // black: the downsampling lets a little of the page's white
        // bleed in, but it stays far from the page
        let middle = read.get_pixel(32, 24).0;
        assert!(
            middle[0] < 128,
            "the middle of the secret stays dark, saw {}", middle[0]
        );
        // the straddling block mixes the secret with its page
        let straddling = read.get_pixel(27, 18).0;
        assert_ne!(straddling[0], 0, "the straddling block is not all black");
        assert_ne!(
            straddling[0], 255,
            "the straddling block is not all page"
        );
        assert_eq!(
            read.get_pixel(2, 2),
            &image::Rgba([255, 255, 255, 255]),
            "outside the area nothing changed"
        );
        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    fn the_redact_video_graph_draws_what_time_says() {
        // Two solid boxes: one for its own stretch of time, one for the
        // whole way through.
        let settings = ToolSettings {
            redact_style: Some(RedactStyle::Solid),
            redact_color: Some("ff0000".to_string()),
            redact_areas: vec![
                RedactArea {
                    x_mille: 0,
                    y_mille: 0,
                    w_mille: 500,
                    h_mille: 500,
                    start_ms: Some(1_000),
                    end_ms: Some(2_000),
                },
                RedactArea {
                    x_mille: 500,
                    y_mille: 500,
                    w_mille: 500,
                    h_mille: 500,
                    start_ms: None,
                    end_ms: None,
                },
            ],
            ..ToolSettings::default()
        };
        let graph = redact_video_filter(&settings, 80, 60).expect("the graph built");
        // the first box stands from one second to two
        assert!(
            graph.contains(
                "drawbox=x=0:y=0:w=40:h=30:color=0xff0000@1:t=fill:enable='between(t,1.000,2.000)'[r0];"
            ),
            "the ranged box is not drawn the way the program reads it: {graph}"
        );
        // the second covers the whole way through, so no time rule, and
        // its stream is the one the graph hands out
        assert!(
            graph.contains("[r0]drawbox=x=40:y=30:w=40:h=30:color=0xff0000@1:t=fill[vout]"),
            "the whole-way box is not drawn: {graph}"
        );
        assert!(!graph.contains("boxblur"), "no blur filter is borrowed: {graph}");
    }

    #[test]
    fn the_redact_video_graph_smears_and_blocks_with_what_the_program_has() {
        // This build of the program carries no blur filter at all (the
        // real test of the program proves the parts), so the smear is
        // the crop made small and laid back large, and the squares are
        // the same thing lifted with the nearest-neighbor flag.
        let settings = ToolSettings {
            redact_style: Some(RedactStyle::Blur),
            redact_areas: vec![RedactArea {
                x_mille: 250,
                y_mille: 250,
                w_mille: 500,
                h_mille: 500,
                start_ms: None,
                end_ms: None,
            }],
            ..ToolSettings::default()
        };
        let graph = redact_video_filter(&settings, 80, 60).expect("the graph built");
        // the crop is the rectangle; the effect is 16% of the smaller
        // side (9.6 of 60), so the small copy is four by three
        assert!(
            graph.contains(
                "crop=x=20:y=15:w=40:h=30,scale=4:3,scale=40:30[sm0];"
            ),
            "the smear is not made the way it was decided: {graph}"
        );
        assert!(
            graph.contains("[m0][sm0]overlay=x=20:y=15[vout]"),
            "the smear is not laid back where it came from: {graph}"
        );
        assert!(!graph.contains("boxblur"), "no missing filter is asked for: {graph}");

        let settings = ToolSettings {
            redact_style: Some(RedactStyle::Pixelate),
            ..settings
        };
        let graph = redact_video_filter(&settings, 80, 60).expect("the graph built");
        assert!(
            graph.contains(
                "crop=x=20:y=15:w=40:h=30,scale=5:4,scale=40:30:flags=neighbor[px0];"
            ),
            "the squares are not cut: {graph}"
        );
        assert!(!graph.contains("pixelize"), "no missing filter is asked for: {graph}");
    }

    #[test]
    fn a_video_covered_by_nothing_says_something() {
        // A rectangle at the very edge covers no pixel of the picture.
        let settings = ToolSettings {
            redact_areas: vec![RedactArea {
                x_mille: 1000,
                y_mille: 1000,
                w_mille: 500,
                h_mille: 500,
                start_ms: None,
                end_ms: None,
            }],
            ..ToolSettings::default()
        };
        let outcome = redact_video_filter(&settings, 80, 60);
        assert_eq!(outcome, Err(ToolError::NothingToDo));
    }

    #[test]
    fn the_redact_video_args_carry_the_graph_and_the_maps() {
        let source = crate::media::manifest::Manifest::get()
            .format("mp4")
            .expect("mp4 is in the manifest");
        let arguments = redact_video_args(
            Path::new("in.mp4"),
            &source,
            Path::new("in.redact-part.mp4"),
            "[0:v]drawbox=x=0:y=0:w=1:h=1:color=0x000000@1:t=fill[vout]",
        );
        let words: Vec<String> = arguments
            .iter()
            .map(|word| word.to_string_lossy().into_owned())
            .collect();
        let graph_at = words
            .iter()
            .position(|w| w == "-filter_complex")
            .expect("the graph was asked for");
        assert!(
            words[graph_at + 1].contains("drawbox"),
            "the graph rides in one word: {}", words[graph_at + 1]
        );
        let out_at = words
            .iter()
            .position(|word| word == "[vout]")
            .expect("the covered stream is mapped");
        assert_eq!(words[out_at - 1], "-map", "the map names the stream");
        assert!(
            words.iter().any(|word| word == "0:a?"),
            "the sound is mapped when it is there"
        );
        let quality_at = words
            .iter()
            .position(|word| word == "-q:v")
            .expect("the picture quality was set");
        assert_eq!(words[quality_at + 1], "5");
    }

    #[test]
    fn the_bleep_tone_sings_at_the_asked_pitch() {
        // A thousand samples a second and two hundred and fifty hertz:
        // one tone wave every four frames, at half of full scale.
        let mut samples = vec![0i16; 4];
        bleep_over(&mut samples, 1, 0, 4, 1000, 250);
        assert_eq!(
            samples,
            vec![0, 16384, 0, -16384],
            "the tone is a half-scale sine at the asked pitch"
        );
    }

    #[test]
    fn the_bleep_replaces_what_it_stands_over_and_nothing_else() {
        let mut samples = vec![7i16; 12];
        bleep_over(&mut samples, 1, 1, 3, 1000, 250);
        assert_eq!(
            samples,
            vec![
                7, 16384, 0, //
                7, 7, 7, 7, 7, 7, 7, 7, 7,
            ],
            "the tone stands over its stretch and nothing else"
        );
    }

    #[test]
    fn the_bleep_tone_sings_the_same_in_every_channel() {
        // Two channels of one frame at a quarter of the tone's wave:
        // both ears hear the same instant.
        let mut samples = vec![0i16; 2];
        bleep_over(&mut samples, 2, 0, 1, 1000, 250);
        assert_eq!(
            samples,
            vec![0, 0],
            "frame nought stands at the wave's own nought"
        );

        let mut samples = vec![0i16; 4];
        bleep_over(&mut samples, 2, 1, 2, 1000, 250);
        assert_eq!(samples, vec![0, 0, 16384, 16384]);
    }

    #[test]
    fn the_bleep_frequency_is_clamped_where_the_slider_stands() {
        assert_eq!(ToolSettings::default().bleep_frequency_hz(), 1000);
        assert_eq!(
            ToolSettings {
                bleep_frequency: Some(200),
                ..ToolSettings::default()
            }
            .bleep_frequency_hz(),
            300
        );
        assert_eq!(
            ToolSettings {
                bleep_frequency: Some(5000),
                ..ToolSettings::default()
            }
            .bleep_frequency_hz(),
            2000
        );
    }

    #[test]
    fn a_bleep_off_the_end_of_the_sound_is_no_bleep_at_all() {
        let range = BleepRange {
            start_ms: 3000,
            end_ms: 9000,
        };
        // what hangs off the end is cut away, nothing more
        assert_eq!(range.frame_span(8000, 1000), (3000, 8000));
        // a range past the end covers nothing
        let range = BleepRange {
            start_ms: 9000,
            end_ms: 9500,
        };
        assert_eq!(range.frame_span(8000, 1000), (8000, 8000));
    }

    #[test]
    fn a_sound_with_no_bleeps_is_refused_and_said_so() {
        let tools = Tools {
            ffmpeg_tools: crate::media::locator::Tools {
                ffmpeg: PathBuf::from("ffmpeg"),
                ffprobe: PathBuf::from("ffprobe"),
            },
            publisher: CopyPublisher,
        };
        let request = Request {
            operation: "tools.bleep".to_string(),
            inputs: vec![PathBuf::from("tone.wav")],
            settings: ToolSettings::default(),
            ..Default::default()
        };
        let outcome = tools.run(&request, &Selection::inspect(&[PathBuf::from("tone.wav")]));
        match outcome {
            Outcome::Failed {
                reason: Failure::InvalidInput(message),
            } => {
                assert_eq!(message, "there are no bleep ranges");
            }
            other => panic!("the bleep was not refused for saying nothing: {other:?}"),
        }
    }

    #[test]
    #[ignore = "needs the conversion program"]
    fn a_sound_is_bleeped_the_way_the_ranges_mark() {
        let ffmpeg = Path::new("vendor").join("ffmpeg").join("ffmpeg.exe");
        if !ffmpeg.is_file() {
            eprintln!("the program is not there; this test needs it");
            return;
        }
        let scratch = std::env::temp_dir().join("open-tangerine-bleep-test");
        std::fs::create_dir_all(&scratch).expect("the scratch folder was made");
        let input = scratch.join("tone.wav");

        // One second of a dull constant tone in both ears: eight
        // thousand frames of five thousands.
        let flat = vec![5000i16; 16000];
        let wav = crate::media::audio::encode_wav_bytes(&flat, 8000, 2);
        std::fs::write(&input, wav).expect("tone.wav written");

        let tools = Tools {
            ffmpeg_tools: crate::media::locator::Tools {
                ffmpeg,
                ffprobe: Path::new("vendor").join("ffmpeg").join("ffprobe.exe"),
            },
            publisher: CopyPublisher,
        };
        let request = Request {
            operation: "tools.bleep".to_string(),
            inputs: vec![input.clone()],
            settings: ToolSettings {
                // a fifth of a second to half a second: the tone sings
                // there, at the frequency nobody moved (a thousand)
                bleep_ranges: vec![BleepRange {
                    start_ms: 200,
                    end_ms: 500,
                }],
                ..ToolSettings::default()
            },
            ..Default::default()
        };
        let outcome = tools.run(&request, &Selection::inspect(&[input.clone()]));
        let outputs = match outcome {
            Outcome::Succeeded { outputs } => outputs,
            Outcome::Failed { reason } => panic!("the bleep failed: {reason:?}"),
            Outcome::Cancelled => panic!("the bleep was not asked to stop"),
        };
        let made = outputs.first().expect("the bleeped copy is there");
        assert!(made.is_file(), "the bleeped copy exists");
        assert_eq!(
            made.file_name().and_then(|n| n.to_str()),
            Some("tone bleeped.wav")
        );

        // read it back: what the bleep stood over sings at half scale,
        // and what it stood away from is exactly what was there
        let bytes = std::fs::read(made).expect("the bleeped copy reads");
        let (samples, rate) = crate::media::audio::parse_wav_samples(&bytes).expect("parse wav");
        assert_eq!(rate, 8000, "the sound keeps its rate");
        assert_eq!(samples.len(), 16000, "the sound keeps its length");
        assert_eq!(samples[0], 5000, "before the bleep nothing changed");
        assert_eq!(samples[15999], 5000, "after the bleep nothing changed");
        let start = 2 * 1600; // two channels, two hundred milliseconds in
        let end = 2 * 4000; // two channels, half a second in
        let peak = samples[start..end]
            .iter()
            .map(|&s| (s as i32).abs())
            .max()
            .unwrap_or(0);
        assert!(
            (peak - 16384).abs() <= 1,
            "the tone stands half of full scale, saw {peak}"
        );
        assert_ne!(
            samples[start],
            5000,
            "the start of the bleep is no longer the sound it was"
        );
        // both ears hear the same instant of the tone
        assert_eq!(samples[start], samples[start + 1]);

        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    fn a_video_redaction_without_areas_is_refused_and_said_so() {
        let tools = Tools {
            ffmpeg_tools: crate::media::locator::Tools {
                ffmpeg: PathBuf::from("ffmpeg"),
                ffprobe: PathBuf::from("ffprobe"),
            },
            publisher: CopyPublisher,
        };
        let request = Request {
            operation: "tools.redact_video".to_string(),
            inputs: vec![PathBuf::from("clip.mp4")],
            settings: ToolSettings::default(),
            ..Default::default()
        };
        let outcome = tools.run(&request, &Selection::inspect(&[PathBuf::from("clip.mp4")]));
        match outcome {
            Outcome::Failed {
                reason: Failure::InvalidInput(message),
            } => {
                assert_eq!(message, "there are no redaction areas");
            }
            other => panic!("the redaction was not refused for saying nothing: {other:?}"),
        }
    }

    #[test]
    #[ignore = "needs the conversion program"]
    fn a_video_is_redacted_the_way_the_styles_ask() {
        let folder = Path::new("vendor").join("ffmpeg");
        let ffmpeg = folder.join("ffmpeg.exe");
        let ffprobe = folder.join("ffprobe.exe");
        if !ffmpeg.is_file() || !ffprobe.is_file() {
            eprintln!("the program is not there; this test needs it");
            return;
        }
        let scratch = std::env::temp_dir().join("open-tangerine-redact-video-test");
        std::fs::create_dir_all(&scratch).expect("the scratch folder was made");
        let input = make_test_video(&scratch);

        // A smear over the left half the whole way, over the right half
        // the whole way, and over the middle strip for one second only â€”
        // three covers in one graph, one style asked (the way the
        // reference asks a style of the whole batch, not of each box).
        let tools = Tools {
            ffmpeg_tools: crate::media::locator::Tools {
                ffmpeg: ffmpeg.clone(),
                ffprobe: ffprobe.clone(),
            },
            publisher: CopyPublisher,
        };
        let request = Request {
            operation: "tools.redact_video".to_string(),
            inputs: vec![input.clone()],
            settings: ToolSettings {
                redact_style: Some(RedactStyle::Blur),
                redact_areas: vec![
                    RedactArea {
                        x_mille: 0,
                        y_mille: 0,
                        w_mille: 500,
                        h_mille: 1000,
                        start_ms: None,
                        end_ms: None,
                    },
                    RedactArea {
                        x_mille: 500,
                        y_mille: 0,
                        w_mille: 500,
                        h_mille: 1000,
                        start_ms: None,
                        end_ms: None,
                    },
                    RedactArea {
                        x_mille: 300,
                        y_mille: 400,
                        w_mille: 400,
                        h_mille: 200,
                        start_ms: Some(1_000),
                        end_ms: Some(2_000),
                    },
                ],
                ..ToolSettings::default()
            },
            ..Default::default()
        };
        let outcome = tools.run(&request, &Selection::inspect(&[input.clone()]));
        let outputs = match outcome {
            Outcome::Succeeded { outputs } => outputs,
            Outcome::Failed { reason } => panic!("the redaction failed: {reason:?}"),
            Outcome::Cancelled => panic!("the redaction was not asked to stop"),
        };
        let made = outputs.first().expect("the redaction is there");
        assert!(made.is_file(), "the covered copy exists");
        assert_eq!(
            made.file_name().and_then(|n| n.to_str()),
            Some("clip redacted.mp4")
        );
        let size = video_size(&ffprobe, made);
        assert_eq!(size, (160, 120), "the covered video keeps its shape");
        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    #[ignore = "needs the conversion program"]
    fn a_trim_runs_the_whole_way_from_the_wheel_path() {
        // A trim keeps the format it was given, and there was a time when
        // the handler only knew how to answer a named format Ã¢â‚¬â€ the trim
        // was refused before it began. This walks the very path the wheel
        // drives, so the refusal can never come back unnoticed.
        let folder = Path::new("vendor").join("ffmpeg");
        let ffmpeg = folder.join("ffmpeg.exe");
        if !ffmpeg.is_file() {
            eprintln!("the program is not there; this test needs it");
            return;
        }
        let scratch = std::env::temp_dir().join("open-tangerine-trim-wheel-test");
        std::fs::create_dir_all(&scratch).expect("the scratch folder was made");
        let input = make_test_video(&scratch);

        let tools = Tools {
            ffmpeg_tools: crate::media::locator::Tools {
                ffmpeg,
                ffprobe: Path::new("vendor").join("ffmpeg").join("ffprobe.exe"),
            },
            publisher: CopyPublisher,
        };
        let request = Request {
            operation: "tools.trim".to_string(),
            inputs: vec![input.clone()],
            settings: ToolSettings {
                start_ms: Some(0),
                end_ms: Some(1_000),
                ..Default::default()
            },
            ..Default::default()
        };
        let outcome = tools.run(&request, &Selection::inspect(&[input.clone()]));
        match outcome {
            Outcome::Succeeded { outputs } => {
                assert_eq!(outputs.len(), 1, "one file in, one file out");
                assert!(outputs.first().is_some_and(|made| made.is_file()));
            }
            Outcome::Failed { reason } => panic!("the trim failed: {:?}", reason),
            Outcome::Cancelled => panic!("the trim was not asked to stop"),
        }
        let _ = std::fs::remove_dir_all(&scratch);
    }

    /// A picture with detail in it â€” noise, so no encoder gets it for
    /// free â€” written as a JPEG at the quality the test asks for.
    fn make_noisy_jpeg(scratch: &Path, name: &str, quality: u8) -> PathBuf {
        let mut picture = image::DynamicImage::new_rgb8(320, 240);
        let mut seed = 0x2545F491u32;
        for (x, y, pixel) in picture.as_mut_rgb8().expect("rgb").enumerate_pixels_mut() {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let (x, y) = (x as u8, y as u8);
            *pixel = image::Rgb([seed as u8, (seed >> 8) as u8 ^ x, (seed >> 16) as u8 ^ y]);
        }
        let input = scratch.join(name);
        write_jpeg(&picture, &input, quality).expect("the picture was made");
        input
    }

    #[test]
    #[ignore = "needs the picture library and nothing more"]
    fn a_jpeg_is_made_lighter_at_the_preset() {
        let scratch = std::env::temp_dir().join("open-tangerine-compress-jpeg-test");
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(&scratch).expect("the scratch folder was made");
        // The original is written at the top quality, so the balanced
        // preset has room to make it lighter.
        let input = make_noisy_jpeg(&scratch, "photo.jpg", 100);
        let weight = std::fs::metadata(&input).expect("the original weighs").len();

        let tools = Tools {
            ffmpeg_tools: crate::media::locator::Tools {
                ffmpeg: PathBuf::from("ffmpeg"),
                ffprobe: PathBuf::from("ffprobe"),
            },
            publisher: CopyPublisher,
        };
        let request = Request {
            operation: "tools.compress".to_string(),
            inputs: vec![input.clone()],
            settings: ToolSettings::default(),
            ..Default::default()
        };
        let outcome = tools.run(&request, &Selection::inspect(&[input.clone()]));
        let outputs = match outcome {
            Outcome::Succeeded { outputs } => outputs,
            Outcome::Failed { reason } => panic!("the compress failed: {:?}", reason),
            Outcome::Cancelled => panic!("the compress was not asked to stop"),
        };
        assert_eq!(outputs.len(), 1, "one file in, one file out");
        let made = outputs.first().expect("the compress is there");
        assert!(made.is_file(), "the lighter file exists");
        assert_ne!(made, &input, "the original stands untouched");
        let again = image::open(made).expect("the result is a picture");
        assert_eq!((again.width(), again.height()), (320, 240), "the shape is kept");
        let lighter = std::fs::metadata(made).expect("the result weighs").len();
        assert!(lighter < weight, "{lighter} should be under {weight}");
        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    #[ignore = "needs the picture library and nothing more"]
    fn a_jpeg_searches_for_the_quality_that_fits() {
        let scratch = std::env::temp_dir().join("open-tangerine-compress-target-test");
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(&scratch).expect("the scratch folder was made");
        let input = make_noisy_jpeg(&scratch, "photo.jpg", 100);

        let tools = Tools {
            ffmpeg_tools: crate::media::locator::Tools {
                ffmpeg: PathBuf::from("ffmpeg"),
                ffprobe: PathBuf::from("ffprobe"),
            },
            publisher: CopyPublisher,
        };
        let target = std::fs::metadata(&input).expect("the original weighs").len() / 2;
        let request = Request {
            operation: "tools.compress".to_string(),
            inputs: vec![input.clone()],
            settings: ToolSettings {
                target_bytes: Some(target),
                ..Default::default()
            },
            ..Default::default()
        };
        let outcome = tools.run(&request, &Selection::inspect(&[input.clone()]));
        let outputs = match outcome {
            Outcome::Succeeded { outputs } => outputs,
            Outcome::Failed { reason } => panic!("the search failed: {:?}", reason),
            Outcome::Cancelled => panic!("the search was not asked to stop"),
        };
        let made = outputs.first().expect("the compress is there");
        let weight = std::fs::metadata(made).expect("the result weighs").len();
        assert!(
            weight <= target,
            "{weight} should fit under the asked {target}"
        );
        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    #[ignore = "needs the picture library and nothing more"]
    fn a_picture_asked_for_an_impossible_size_says_so() {
        let scratch = std::env::temp_dir().join("open-tangerine-compress-small-test");
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(&scratch).expect("the scratch folder was made");
        let input = scratch.join("photo.png");
        image::DynamicImage::new_rgb8(400, 300)
            .save(&input)
            .expect("the picture was made");

        let tools = Tools {
            ffmpeg_tools: crate::media::locator::Tools {
                ffmpeg: PathBuf::from("ffmpeg"),
                ffprobe: PathBuf::from("ffprobe"),
            },
            publisher: CopyPublisher,
        };
        // A picture keeps being made less until there is almost nothing
        // left of it; sixty-four bytes is not a size, it is a wish.
        let request = Request {
            operation: "tools.compress".to_string(),
            inputs: vec![input.clone()],
            settings: ToolSettings {
                target_bytes: Some(64),
                ..Default::default()
            },
            ..Default::default()
        };
        let outcome = tools.run(&request, &Selection::inspect(&[input.clone()]));
        match outcome {
            Outcome::Failed { reason } => {
                assert!(
                    reason.message().contains("smaller than the file can be made"),
                    "the failure says what went wrong: {:?}",
                    reason
                );
            }
            Outcome::Succeeded { .. } => panic!("an impossible size was honoured"),
            Outcome::Cancelled => panic!("the compress was not asked to stop"),
        }
        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    #[ignore = "needs the conversion program"]
    fn a_video_is_made_lighter_at_a_target_size() {
        let folder = Path::new("vendor").join("ffmpeg");
        let ffmpeg = folder.join("ffmpeg.exe");
        let ffprobe = folder.join("ffprobe.exe");
        if !ffmpeg.is_file() || !ffprobe.is_file() {
            eprintln!("the program is not there; this test needs it");
            return;
        }
        let scratch = std::env::temp_dir().join("open-tangerine-compress-video-test");
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(&scratch).expect("the scratch folder was made");
        // A video with detail in it â€” noise, so the encoder has to spend
        // real bits on it, and a target of half its weight stands above
        // the floors the bitrate keeps.
        let raw = scratch.join("noise.raw");
        let mut seed = 0x2545F491u32;
        let mut pixels = Vec::with_capacity(160 * 120 * 30 * 3);
        for _ in 0..160 * 120 * 30 {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            pixels.extend_from_slice(&[seed as u8, (seed >> 8) as u8, (seed >> 16) as u8]);
        }
        std::fs::write(&raw, &pixels).expect("the pixels could be written");
        let input = scratch.join("clip.mp4");
        let making: Vec<OsString> = [
            "-hide_banner",
            "-nostdin",
            "-y",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgb24",
            "-s",
            "160x120",
            "-r",
            "10",
            "-i",
        ]
        .iter()
        .map(|word| OsString::from(*word))
        .chain([
            raw.as_os_str().to_os_string(),
            OsString::from("-c:v"),
            OsString::from("mpeg4"),
            OsString::from("-pix_fmt"),
            OsString::from("yuv420p"),
            OsString::from("-f"),
            OsString::from("mov"),
            input.as_os_str().to_os_string(),
        ])
        .collect();
        let made_video = SystemRunner
            .run(&ffmpeg, &making, None)
            .expect("the video was made");
        assert!(made_video.succeeded(), "the test video was not made: {}", made_video.stderr);
        let input = input;
        let weight = std::fs::metadata(&input).expect("the original weighs").len();

        let tools = Tools {
            ffmpeg_tools: crate::media::locator::Tools {
                ffmpeg,
                ffprobe: ffprobe.clone(),
            },
            publisher: CopyPublisher,
        };
        let request = Request {
            operation: "tools.compress".to_string(),
            inputs: vec![input.clone()],
            settings: ToolSettings {
                target_bytes: Some(weight / 2),
                ..Default::default()
            },
            ..Default::default()
        };
        let outcome = tools.run(&request, &Selection::inspect(&[input.clone()]));
        let outputs = match outcome {
            Outcome::Succeeded { outputs } => outputs,
            Outcome::Failed { reason } => panic!("the compress failed: {:?}", reason),
            Outcome::Cancelled => panic!("the compress was not asked to stop"),
        };
        let made = outputs.first().expect("the compress is there").clone();
        assert!(made.is_file(), "the lighter video exists");
        let (w, h) = video_size(&ffprobe, &made);
        assert_eq!((w, h), (160, 120), "the shape is kept");
        let lighter = std::fs::metadata(&made).expect("the result weighs").len();
        assert!(
            lighter < weight,
            "{lighter} should be under the original's {weight}"
        );
        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    #[ignore = "needs the conversion program"]
    fn a_sound_is_made_lighter_at_the_strong_preset() {
        let folder = Path::new("vendor").join("ffmpeg");
        let ffmpeg = folder.join("ffmpeg.exe");
        if !ffmpeg.is_file() {
            eprintln!("the program is not there; this test needs it");
            return;
        }
        let scratch = std::env::temp_dir().join("open-tangerine-compress-audio-test");
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(&scratch).expect("the scratch folder was made");
        // A second of a tone, written as an MP3 at the balanced rate.
        let wav = scratch.join("tone.wav");
        let rate: u32 = 44100;
        let data_len: u32 = rate * 2;
        let mut wav_bytes = Vec::with_capacity(44 + data_len as usize);
        wav_bytes.extend_from_slice(b"RIFF");
        wav_bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
        wav_bytes.extend_from_slice(b"WAVEfmt ");
        wav_bytes.extend_from_slice(&16u32.to_le_bytes());
        wav_bytes.extend_from_slice(&1u16.to_le_bytes());
        wav_bytes.extend_from_slice(&1u16.to_le_bytes());
        wav_bytes.extend_from_slice(&rate.to_le_bytes());
        wav_bytes.extend_from_slice(&(rate * 2).to_le_bytes());
        wav_bytes.extend_from_slice(&2u16.to_le_bytes());
        wav_bytes.extend_from_slice(&16u16.to_le_bytes());
        wav_bytes.extend_from_slice(b"data");
        wav_bytes.extend_from_slice(&data_len.to_le_bytes());
        for i in 0..rate {
            let sample: i16 = if (i / 50) % 2 == 0 { 10000 } else { -10000 };
            wav_bytes.extend_from_slice(&sample.to_le_bytes());
        }
        std::fs::write(&wav, wav_bytes).expect("the tone could be written");
        let input = scratch.join("tone.mp3");
        let making: Vec<OsString> = [
            "-hide_banner",
            "-nostdin",
            "-y",
            "-f",
            "wav",
            "-i",
        ]
        .iter()
        .map(|word| OsString::from(*word))
        .chain([
            wav.into_os_string(),
            OsString::from("-c:a"),
            OsString::from("libmp3lame"),
            OsString::from("-b:a"),
            OsString::from("128k"),
            OsString::from("-f"),
            OsString::from("mp3"),
            input.as_os_str().to_os_string(),
        ])
        .collect();
        let made = SystemRunner.run(&ffmpeg, &making, None).expect("the tone was made");
        assert!(made.succeeded(), "the test sound was not made: {}", made.stderr);
        let weight = std::fs::metadata(&input).expect("the original weighs").len();

        let tools = Tools {
            ffmpeg_tools: crate::media::locator::Tools {
                ffmpeg,
                ffprobe: Path::new("vendor").join("ffmpeg").join("ffprobe.exe"),
            },
            publisher: CopyPublisher,
        };
        let request = Request {
            operation: "tools.compress".to_string(),
            inputs: vec![input.clone()],
            settings: ToolSettings {
                compress_strong: true,
                ..Default::default()
            },
            ..Default::default()
        };
        let outcome = tools.run(&request, &Selection::inspect(&[input.clone()]));
        let outputs = match outcome {
            Outcome::Succeeded { outputs } => outputs,
            Outcome::Failed { reason } => panic!("the compress failed: {:?}", reason),
            Outcome::Cancelled => panic!("the compress was not asked to stop"),
        };
        let made = outputs.first().expect("the compress is there").clone();
        assert!(made.is_file(), "the lighter sound exists");
        let lighter = std::fs::metadata(&made).expect("the result weighs").len();
        assert!(
            lighter < weight,
            "{lighter} should be under the original's {weight}"
        );
        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    fn metadata_args_with_remove_all_clears_metadata_and_chapters() {
        let input = Path::new("song.mp3");
        let output = Path::new("song.meta-part");
        let settings = ToolSettings {
            metadata_remove_all: true,
            ..Default::default()
        };
        let args = metadata_args(input, output, Some("mp3"), &settings);
        let joined = args
            .iter()
            .map(|s| s.to_string_lossy())
            .collect::<Vec<_>>()
            .join(" ");
        assert!(joined.contains("-map 0 -c copy"), "args must stream copy: {joined}");
        assert!(joined.contains("-map_metadata -1"), "args must clear metadata: {joined}");
        assert!(joined.contains("-map_chapters -1"), "args must clear chapters: {joined}");
    }

    #[test]
    fn metadata_args_with_tags_sets_metadata_and_copies_streams() {
        let input = Path::new("clip.mp4");
        let output = Path::new("clip.meta-part");
        let settings = ToolSettings {
            metadata_tags: vec![
                ("title".to_string(), "Sample Video".to_string()),
                ("artist".to_string(), "Open Tangerine".to_string()),
            ],
            ..Default::default()
        };
        let args = metadata_args(input, output, Some("mp3"), &settings);
        let joined = args
            .iter()
            .map(|s| s.to_string_lossy())
            .collect::<Vec<_>>()
            .join(" ");
        assert!(joined.contains("-map 0 -c copy"), "args must stream copy: {joined}");
        assert!(joined.contains("-map_metadata 0"), "args must keep stream metadata: {joined}");
        assert!(joined.contains("-metadata title=Sample Video"), "title must be set: {joined}");
        assert!(joined.contains("-metadata artist=Open Tangerine"), "artist must be set: {joined}");
    }

    #[test]
    fn metadata_final_names_adds_suffix_and_numbers() {
        let original = Path::new("C:\\music\\track.mp3");
        let names = metadata_final_names(original, "clean", "mp3");
        assert_eq!(names[0], PathBuf::from("C:\\music\\track clean.mp3"));
        assert_eq!(names[1], PathBuf::from("C:\\music\\track clean (2).mp3"));
        assert_eq!(names[2], PathBuf::from("C:\\music\\track clean (3).mp3"));

        let edited_names = metadata_final_names(original, "edited", "mp3");
        assert_eq!(edited_names[0], PathBuf::from("C:\\music\\track edited.mp3"));
        assert_eq!(edited_names[1], PathBuf::from("C:\\music\\track edited (2).mp3"));
    }

    fn make_wav_bytes(duration_secs: f32, rate: u32) -> Vec<u8> {
        let sample_count = (duration_secs * rate as f32) as u32;
        let data_len = sample_count * 2;
        let mut wav_bytes = Vec::with_capacity(44 + data_len as usize);
        wav_bytes.extend_from_slice(b"RIFF");
        wav_bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
        wav_bytes.extend_from_slice(b"WAVEfmt ");
        wav_bytes.extend_from_slice(&16u32.to_le_bytes());
        wav_bytes.extend_from_slice(&1u16.to_le_bytes());
        wav_bytes.extend_from_slice(&1u16.to_le_bytes());
        wav_bytes.extend_from_slice(&rate.to_le_bytes());
        wav_bytes.extend_from_slice(&(rate * 2).to_le_bytes());
        wav_bytes.extend_from_slice(&2u16.to_le_bytes());
        wav_bytes.extend_from_slice(&16u16.to_le_bytes());
        wav_bytes.extend_from_slice(b"data");
        wav_bytes.extend_from_slice(&data_len.to_le_bytes());
        for i in 0..sample_count {
            let sample: i16 = if (i / 50) % 2 == 0 { 10000 } else { -10000 };
            wav_bytes.extend_from_slice(&sample.to_le_bytes());
        }
        wav_bytes
    }

    #[test]
    #[ignore = "runs the real program to edit tags on audio"]
    fn real_metadata_edit_on_audio_sets_tags() {
        let ffmpeg = Path::new("vendor").join("ffmpeg").join("ffmpeg.exe");
        if !ffmpeg.is_file() {
            return;
        }
        let scratch = std::env::temp_dir().join(format!("ot-meta-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&scratch);
        let input = scratch.join("tone.wav");

        let pcm = make_wav_bytes(1.0, 44100);
        std::fs::write(&input, pcm).expect("tone.wav written");

        let tools = Tools {
            ffmpeg_tools: crate::media::locator::Tools {
                ffmpeg,
                ffprobe: Path::new("vendor").join("ffmpeg").join("ffprobe.exe"),
            },
            publisher: CopyPublisher,
        };
        let request = Request {
            operation: "tools.metadata".to_string(),
            inputs: vec![input.clone()],
            settings: ToolSettings {
                metadata_tags: vec![("title".to_string(), "Tone Test".to_string())],
                ..Default::default()
            },
            ..Default::default()
        };
        let outcome = tools.run(&request, &Selection::inspect(&[input.clone()]));
        let outputs = match outcome {
            Outcome::Succeeded { outputs } => outputs,
            Outcome::Failed { reason } => panic!("metadata edit failed: {:?}", reason),
            Outcome::Cancelled => panic!("cancelled"),
        };
        let made = outputs.first().expect("output exists");
        assert!(made.is_file(), "edited file exists: {:?}", made);
        assert!(made.to_string_lossy().contains("tone edited.wav"));
        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    fn normalize_settings_defaults() {
        let settings = ToolSettings::default();
        assert_eq!(settings.target_lufs(), -16.0);
        assert_eq!(settings.loudness_range(), 11.0);
        assert_eq!(settings.true_peak(), -1.5);

        let custom = ToolSettings {
            normalize_lufs_x10: Some(-140),
            normalize_range_x10: Some(90),
            normalize_true_peak_x10: Some(-10),
            ..Default::default()
        };
        assert_eq!(custom.target_lufs(), -14.0);
        assert_eq!(custom.loudness_range(), 9.0);
        assert_eq!(custom.true_peak(), -1.0);
    }

    #[test]
    #[ignore = "runs the real program to normalize audio volume"]
    fn real_audio_normalize_with_the_program() {
        let ffmpeg = Path::new("vendor").join("ffmpeg").join("ffmpeg.exe");
        if !ffmpeg.is_file() {
            return;
        }
        let scratch = std::env::temp_dir().join(format!("ot-norm-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&scratch);
        let input = scratch.join("quiet.wav");

        // Make 1-second WAV with amplitude 10000 (~0.305 peak)
        let pcm = make_wav_bytes(1.0, 44100);
        std::fs::write(&input, pcm).expect("quiet.wav written");

        let tools = Tools {
            ffmpeg_tools: crate::media::locator::Tools {
                ffmpeg,
                ffprobe: Path::new("vendor").join("ffmpeg").join("ffprobe.exe"),
            },
            publisher: CopyPublisher,
        };
        let request = Request {
            operation: "tools.normalize".to_string(),
            inputs: vec![input.clone()],
            settings: ToolSettings {
                normalize_true_peak_x10: Some(-15),
                ..Default::default()
            },
            ..Default::default()
        };
        let outcome = tools.run(&request, &Selection::inspect(&[input.clone()]));
        let outputs = match outcome {
            Outcome::Succeeded { outputs } => outputs,
            Outcome::Failed { reason } => panic!("normalize failed: {:?}", reason),
            Outcome::Cancelled => panic!("cancelled"),
        };
        let made = outputs.first().expect("output exists");
        assert!(made.is_file(), "normalized file exists: {:?}", made);
        assert!(made.to_string_lossy().contains("quiet normalized.wav"));

        // Check that the output amplitude was increased towards target peak ~0.841 (-1.5 dBTP)
        let bytes = std::fs::read(made).expect("read normalized wav");
        let (samples, _) = crate::media::audio::parse_wav_samples(&bytes).expect("parse wav");
        let peak = samples.iter().map(|&s| (s as i32).abs()).max().unwrap_or(0) as f64 / 32767.0;
        assert!(
            (peak - 0.8414).abs() < 0.05,
            "peak amplitude after normalization was {peak}, expected ~0.8414"
        );

        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    fn channels_settings_defaults() {
        let settings = ToolSettings::default();
        assert_eq!(settings.channel_mode(), "stereo");
        assert_eq!(settings.left_gain_factor(), 1.0);
        assert_eq!(settings.right_gain_factor(), 1.0);

        let custom = ToolSettings {
            channel_mode: Some("mono".to_string()),
            channel_left_gain: Some(150),
            channel_right_gain: Some(50),
            ..Default::default()
        };
        assert_eq!(custom.channel_mode(), "mono");
        assert_eq!(custom.left_gain_factor(), 1.5);
        assert_eq!(custom.right_gain_factor(), 0.5);
    }

    #[test]
    #[ignore = "runs the real program to convert audio channels"]
    fn real_audio_channels_with_the_program() {
        let ffmpeg = Path::new("vendor").join("ffmpeg").join("ffmpeg.exe");
        if !ffmpeg.is_file() {
            return;
        }

        let scratch = std::env::temp_dir().join(format!("ot-chan-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&scratch);
        let input = scratch.join("stereo_test.wav");

        // Stereo audio: Left channel has 8000 amplitude, Right has 16000 amplitude
        let mut samples = Vec::new();
        for _ in 0..4000 {
            samples.push(8000i16);
            samples.push(16000i16);
        }
        let pcm = crate::media::audio::encode_wav_bytes(&samples, 44100, 2);
        std::fs::write(&input, pcm).expect("stereo_test.wav written");

        let tools = Tools {
            ffmpeg_tools: crate::media::locator::Tools {
                ffmpeg,
                ffprobe: Path::new("vendor").join("ffmpeg").join("ffprobe.exe"),
            },
            publisher: CopyPublisher,
        };
        let request = Request {
            operation: "tools.channels".to_string(),
            inputs: vec![input.clone()],
            settings: ToolSettings {
                channel_mode: Some("swap".to_string()),
                channel_left_gain: Some(100),
                channel_right_gain: Some(100),
                ..Default::default()
            },
            ..Default::default()
        };
        let outcome = tools.run(&request, &Selection::inspect(&[input.clone()]));
        let outputs = match outcome {
            Outcome::Succeeded { outputs } => outputs,
            Outcome::Failed { reason } => panic!("channels conversion failed: {:?}", reason),
            Outcome::Cancelled => panic!("cancelled"),
        };
        let made = outputs.first().expect("output exists");
        assert!(made.is_file(), "converted file exists: {:?}", made);
        assert!(made.to_string_lossy().contains("stereo_test channels.wav"));

        // Verify channels swapped: L should now be 16000, R should be 8000
        let bytes = std::fs::read(made).expect("read swapped wav");
        let parsed = crate::media::audio::parse_wav_audio(&bytes).expect("parse wav");
        assert_eq!(parsed.channels, 2);
        assert_eq!(parsed.samples[0], 16000);
        assert_eq!(parsed.samples[1], 8000);

        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    fn merge_requires_at_least_two_inputs() {
        let tools = Tools {
            ffmpeg_tools: crate::media::locator::Tools {
                ffmpeg: PathBuf::from("ffmpeg"),
                ffprobe: PathBuf::from("ffprobe"),
            },
            publisher: CopyPublisher,
        };
        let request = Request {
            operation: "tools.merge".to_string(),
            inputs: vec![PathBuf::from("one.mp4")],
            ..Default::default()
        };
        let outcome = tools.run(&request, &Selection::inspect(&[PathBuf::from("one.mp4")]));
        assert!(matches!(outcome, Outcome::Failed { .. }));
    }

    #[test]
    #[ignore = "needs pdfium.dll"]
    fn merge_two_pdfs_produces_merged_pdf() {
        if crate::media::pdf::library().is_none() {
            eprintln!("pdfium.dll is not there; this test needs it");
            return;
        }
        let scratch1 = std::env::temp_dir().join(format!("ot-pdf-m1-{}", std::process::id()));
        let scratch2 = std::env::temp_dir().join(format!("ot-pdf-m2-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&scratch1);
        let _ = std::fs::create_dir_all(&scratch2);
        let pdf1 = scratch1.join("doc_a.pdf");
        let pdf2 = scratch2.join("doc_b.pdf");

        // Make two valid PDFs
        let doc_bytes = {
            let content = b"BT /F1 12 Tf 20 50 Td (Page) Tj ET";
            format!(
                "%PDF-1.4\n\
                 1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj\n\
                 2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 >> endobj\n\
                 3 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 200 100] \
                 /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >> endobj\n\
                 4 0 obj << /Length {} >> stream\n{}\nendstream endobj\n\
                 5 0 obj << /Type /Font /Subtype /Type1 /BaseFont /Helvetica >> endobj\n\
                 trailer << /Root 1 0 R /Size 6 >>",
                content.len(),
                String::from_utf8_lossy(content)
            )
        };
        std::fs::write(&pdf1, &doc_bytes).expect("pdf1 written");
        std::fs::write(&pdf2, &doc_bytes).expect("pdf2 written");

        let tools = Tools {
            ffmpeg_tools: crate::media::locator::Tools {
                ffmpeg: PathBuf::from("ffmpeg"),
                ffprobe: PathBuf::from("ffprobe"),
            },
            publisher: CopyPublisher,
        };
        let request = Request {
            operation: "tools.merge".to_string(),
            inputs: vec![pdf1.clone(), pdf2.clone()],
            ..Default::default()
        };
        let outcome = tools.run(&request, &Selection::inspect(&[pdf1.clone(), pdf2.clone()]));
        let outputs = match outcome {
            Outcome::Succeeded { outputs } => outputs,
            Outcome::Failed { reason } => panic!("pdf merge failed: {:?}", reason),
            Outcome::Cancelled => panic!("cancelled"),
        };
        let made = outputs.first().expect("output exists");
        assert!(made.is_file(), "merged file exists: {:?}", made);
        assert!(made.to_string_lossy().contains("doc_a merged.pdf"));

        let pages = crate::media::pdf::render_pages(made, 72, crate::media::pdf::Encoding::Png)
            .expect("rendered pages");
        assert_eq!(pages.len(), 2, "merged document has 2 pages");

        let _ = std::fs::remove_dir_all(&scratch1);
        let _ = std::fs::remove_dir_all(&scratch2);
    }

    #[test]
    fn organize_pdf_empty_inputs_fails() {
        let tools = Tools {
            ffmpeg_tools: crate::media::locator::Tools {
                ffmpeg: PathBuf::from("ffmpeg"),
                ffprobe: PathBuf::from("ffprobe"),
            },
            publisher: CopyPublisher,
        };
        let request = Request {
            operation: "tools.organize_pdf".to_string(),
            inputs: vec![],
            ..Default::default()
        };
        let outcome = tools.run(&request, &Selection::inspect(&[]));
        assert!(matches!(outcome, Outcome::Failed { .. }));
    }

    #[test]
    #[ignore = "needs pdfium.dll"]
    fn organize_pdf_assembles_selected_pages_with_rotation() {
        if crate::media::pdf::library().is_none() {
            eprintln!("pdfium.dll is not there; this test needs it");
            return;
        }
        let scratch = std::env::temp_dir().join(format!("ot-pdf-org-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&scratch);
        let pdf = scratch.join("my_doc.pdf");

        let doc_bytes = {
            let content = b"BT /F1 12 Tf 20 50 Td (Page) Tj ET";
            format!(
                "%PDF-1.4\n\
                 1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj\n\
                 2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 >> endobj\n\
                 3 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 200 100] \
                 /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >> endobj\n\
                 4 0 obj << /Length {} >> stream\n{}\nendstream endobj\n\
                 5 0 obj << /Type /Font /Subtype /Type1 /BaseFont /Helvetica >> endobj\n\
                 trailer << /Root 1 0 R /Size 6 >>",
                content.len(),
                String::from_utf8_lossy(content)
            )
        };
        std::fs::write(&pdf, &doc_bytes).expect("pdf written");

        let tools = Tools {
            ffmpeg_tools: crate::media::locator::Tools {
                ffmpeg: PathBuf::from("ffmpeg"),
                ffprobe: PathBuf::from("ffprobe"),
            },
            publisher: CopyPublisher,
        };
        let request = Request {
            operation: "tools.organize_pdf".to_string(),
            inputs: vec![pdf.clone()],
            settings: ToolSettings {
                pdf_pages: vec![(0, 0, 90), (0, 0, 0)],
                ..Default::default()
            },
        };
        let outcome = tools.run(&request, &Selection::inspect(&[pdf.clone()]));
        let outputs = match outcome {
            Outcome::Succeeded { outputs } => outputs,
            Outcome::Failed { reason } => panic!("organize pdf failed: {:?}", reason),
            Outcome::Cancelled => panic!("cancelled"),
        };
        let made = outputs.first().expect("output exists");
        assert!(made.is_file(), "organized file exists: {:?}", made);
        assert!(made.to_string_lossy().contains("my_doc organized.pdf"));

        let pages = crate::media::pdf::render_pages(made, 72, crate::media::pdf::Encoding::Png)
            .expect("rendered pages");
        assert_eq!(pages.len(), 2, "assembled document has 2 pages");

        // The default is every page of the input, told by the count of the
        // pages, not learned by rendering every page first.
        let _ = std::fs::remove_file(&made);
        let request = Request {
            operation: "tools.organize_pdf".to_string(),
            inputs: vec![pdf.clone()],
            ..Default::default()
        };
        let outcome = tools.run(&request, &Selection::inspect(&[pdf.clone()]));
        let outputs = match outcome {
            Outcome::Succeeded { outputs } => outputs,
            other => panic!("the default organize failed: {other:?}"),
        };
        let default_made = outputs.first().expect("the default output exists");
        let pages =
            crate::media::pdf::render_pages(default_made, 72, crate::media::pdf::Encoding::Png)
                .expect("the default pages rendered");
        assert_eq!(
            pages.len(),
            1,
            "the default takes every page of the input once"
        );

        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    #[ignore = "needs the conversion program to re-time the video"]
    fn merge_two_videos_produces_joined_video() {
        let folder = Path::new("vendor").join("ffmpeg");
        let ffmpeg = folder.join("ffmpeg.exe");
        let ffprobe = folder.join("ffprobe.exe");
        if !ffmpeg.is_file() {
            eprintln!("the program is not there; this test needs it");
            return;
        }

        let scratch = std::env::temp_dir().join(format!("ot-vid-merge-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&scratch);

        let vid1 = scratch.join("clip_a.mp4");
        let vid2 = scratch.join("clip_b.mp4");

        // Make two 160x120 15-frame videos: one red, one blue
        for (vid, color) in [(&vid1, [255u8, 0, 0, 255]), (&vid2, [0u8, 0, 255, 255])] {
            let raw = scratch.join(format!("{}.raw", vid.file_stem().unwrap().to_str().unwrap()));
            std::fs::write(&raw, color.repeat(160 * 120 * 15)).expect("raw video written");
            let args: Vec<OsString> = [
                "-hide_banner",
                "-nostdin",
                "-y",
                "-f", "rawvideo",
                "-pix_fmt", "rgba",
                "-s", "160x120",
                "-r", "15",
                "-i", raw.as_os_str().to_str().unwrap(),
                "-c:v", "mpeg4",
                "-pix_fmt", "yuv420p",
                "-f", "mov",
                "-q:v", "5",
                vid.as_os_str().to_str().unwrap(),
            ]
            .iter()
            .map(|w| OsString::from(*w))
            .collect();
            let made = SystemRunner.run(&ffmpeg, &args, None).expect("video created");
            assert!(made.succeeded(), "FFmpeg failed: status={:?}, stderr={}", made.status, made.stderr);
            let _ = std::fs::remove_file(&raw);
        }

        let tools = Tools {
            ffmpeg_tools: crate::media::locator::Tools {
                ffmpeg: ffmpeg.clone(),
                ffprobe: ffprobe.clone(),
            },
            publisher: CopyPublisher,
        };
        let request = Request {
            operation: "tools.merge".to_string(),
            inputs: vec![vid1.clone(), vid2.clone()],
            ..Default::default()
        };
        let outcome = tools.run(&request, &Selection::inspect(&[vid1.clone(), vid2.clone()]));
        let outputs = match outcome {
            Outcome::Succeeded { outputs } => outputs,
            Outcome::Failed { reason } => panic!("video merge failed: {:?}", reason),
            Outcome::Cancelled => panic!("cancelled"),
        };
        let made = outputs.first().expect("output exists");
        assert!(made.is_file(), "joined file exists: {:?}", made);
        assert!(made.to_string_lossy().contains("clip_a joined.mp4"));

        let (w, h, _) = crate::media::frames::shape_of(&tools.ffmpeg_tools, made).expect("shape");
        assert_eq!((w, h), (160, 120));
        let duration = crate::media::frames::duration_of(&tools.ffmpeg_tools, made).expect("duration");
        // Two 1-second clips should make a ~2-second joined video
        assert!(duration >= 1.5, "duration should be ~2s: {duration}");

        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    fn frame_image_adds_background_and_saves_framed_png() {
        let scratch = std::env::temp_dir().join(format!("ot-frame-img-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&scratch);
        let photo = scratch.join("sample.png");

        let src = image::DynamicImage::new_rgb8(100, 80);
        src.save(&photo).expect("saved sample photo");

        let tools = Tools {
            ffmpeg_tools: crate::media::locator::Tools {
                ffmpeg: PathBuf::from("ffmpeg"),
                ffprobe: PathBuf::from("ffprobe"),
            },
            publisher: CopyPublisher,
        };
        let request = Request {
            operation: "tools.frame_image".to_string(),
            inputs: vec![photo.clone()],
            settings: ToolSettings {
                frame_bg_kind: Some("color".to_string()),
                frame_color_a: Some("FF5A1F".to_string()),
                frame_padding_x1000: Some(100),
                frame_corner_radius: Some(16),
                frame_shadow: Some(12),
                frame_aspect: Some("1:1".to_string()),
                ..Default::default()
            },
            ..Default::default()
        };
        let outcome = tools.run(&request, &Selection::inspect(&[photo.clone()]));
        let outputs = match outcome {
            Outcome::Succeeded { outputs } => outputs,
            Outcome::Failed { reason } => panic!("framing failed: {:?}", reason),
            Outcome::Cancelled => panic!("cancelled"),
        };
        assert_eq!(outputs.len(), 1);
        let out_file = &outputs[0];
        assert!(out_file.is_file(), "output exists: {:?}", out_file);
        assert!(out_file.to_string_lossy().contains("sample framed.png"));

        let framed = image::open(out_file).expect("framed image opens");
        // Aspect 1:1 means width == height
        assert_eq!(framed.width(), framed.height());
        assert!(framed.width() > 100);

        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    fn frame_image_unsupported_kind_fails() {
        let scratch = std::env::temp_dir().join(format!("ot-frame-non-img-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&scratch);
        let txt = scratch.join("notes.txt");
        std::fs::write(&txt, b"hello").unwrap();

        let tools = Tools {
            ffmpeg_tools: crate::media::locator::Tools {
                ffmpeg: PathBuf::from("ffmpeg"),
                ffprobe: PathBuf::from("ffprobe"),
            },
            publisher: CopyPublisher,
        };
        let request = Request {
            operation: "tools.frame_image".to_string(),
            inputs: vec![txt.clone()],
            ..Default::default()
        };
        let outcome = tools.run(&request, &Selection::inspect(&[txt]));
        assert!(matches!(outcome, Outcome::Failed { .. }));

        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    fn audio_to_video_unsupported_kind_fails() {
        let scratch = std::env::temp_dir().join(format!("ot-vis-non-aud-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&scratch);
        let txt = scratch.join("notes.txt");
        std::fs::write(&txt, b"hello").unwrap();

        let tools = Tools {
            ffmpeg_tools: crate::media::locator::Tools {
                ffmpeg: PathBuf::from("ffmpeg"),
                ffprobe: PathBuf::from("ffprobe"),
            },
            publisher: CopyPublisher,
        };
        let request = Request {
            operation: "tools.audio_to_video".to_string(),
            inputs: vec![txt.clone()],
            ..Default::default()
        };
        let outcome = tools.run(&request, &Selection::inspect(&[txt]));
        assert!(matches!(outcome, Outcome::Failed { .. }));

        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    fn audio_to_video_creates_mp4_with_waveform() {
        let folder = Path::new("vendor").join("ffmpeg");
        let ffmpeg = folder.join("ffmpeg.exe");
        let ffprobe = folder.join("ffprobe.exe");
        if !ffmpeg.is_file() || !ffprobe.is_file() {
            return;
        }

        let scratch = std::env::temp_dir().join(format!("ot-vis-mp4-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&scratch);
        let tone = scratch.join("sample.wav");
        let wav = make_wav_bytes(1.0, 44100);
        std::fs::write(&tone, wav).expect("sample.wav written");

        let tools = Tools {
            ffmpeg_tools: crate::media::locator::Tools {
                ffmpeg,
                ffprobe,
            },
            publisher: CopyPublisher,
        };
        let request = Request {
            operation: "tools.audio_to_video".to_string(),
            inputs: vec![tone.clone()],
            settings: ToolSettings {
                visualizer_style: Some("waveform".to_string()),
                visualizer_shape: Some("16:9".to_string()),
                visualizer_accent: Some("#FF5C00".to_string()),
                visualizer_bg: Some("#000000".to_string()),
                ..Default::default()
            },
            ..Default::default()
        };
        let outcome = tools.run(&request, &Selection::inspect(&[tone.clone()]));
        let outputs = match outcome {
            Outcome::Succeeded { outputs } => outputs,
            Outcome::Failed { reason } => panic!("visualizer failed: {:?}", reason),
            Outcome::Cancelled => panic!("cancelled"),
        };
        assert_eq!(outputs.len(), 1);
        let out_file = &outputs[0];
        assert!(out_file.is_file(), "output exists: {:?}", out_file);
        assert!(out_file.to_string_lossy().contains("sample visualizer.mp4"));
        assert!(out_file.metadata().map(|m| m.len() > 1000).unwrap_or(false));

        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    fn collage_operation_combines_photos() {
        let scratch = std::env::temp_dir().join(format!("ot-collage-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&scratch);

        let p1 = scratch.join("photo1.png");
        let p2 = scratch.join("photo2.png");
        let img1 = image::RgbaImage::from_pixel(100, 100, image::Rgba([255, 0, 0, 255]));
        let img2 = image::RgbaImage::from_pixel(100, 100, image::Rgba([0, 255, 0, 255]));
        img1.save(&p1).unwrap();
        img2.save(&p2).unwrap();

        let tools = Tools {
            ffmpeg_tools: crate::media::locator::Tools {
                ffmpeg: PathBuf::from("ffmpeg"),
                ffprobe: PathBuf::from("ffprobe"),
            },
            publisher: CopyPublisher,
        };
        let request = Request {
            operation: "tools.collage".to_string(),
            inputs: vec![p1.clone(), p2.clone()],
            settings: ToolSettings {
                collage_layout: Some("grid".to_string()),
                collage_spacing: Some(10),
                collage_corner_radius: Some(8),
                collage_bg_color: Some("#FFFFFF".to_string()),
                ..Default::default()
            },
        };

        let outcome = tools.run(&request, &Selection::inspect(&[p1.clone(), p2.clone()]));
        let outputs = match outcome {
            Outcome::Succeeded { outputs } => outputs,
            Outcome::Failed { reason } => panic!("collage failed: {:?}", reason),
            Outcome::Cancelled => panic!("cancelled"),
        };
        assert_eq!(outputs.len(), 1);
        let out_file = &outputs[0];
        assert!(out_file.is_file(), "output exists: {:?}", out_file);
        assert!(out_file.to_string_lossy().contains("photo1 collage.jpg"));

        let collage_img = image::open(out_file).expect("collage opens");
        assert!(collage_img.width() >= 1000);
        assert!(collage_img.height() >= 1000);

        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    fn collage_single_photo_fails() {
        let scratch = std::env::temp_dir().join(format!("ot-collage-single-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&scratch);
        let p1 = scratch.join("photo1.png");
        let img1 = image::RgbaImage::from_pixel(100, 100, image::Rgba([255, 0, 0, 255]));
        img1.save(&p1).unwrap();

        let tools = Tools {
            ffmpeg_tools: crate::media::locator::Tools {
                ffmpeg: PathBuf::from("ffmpeg"),
                ffprobe: PathBuf::from("ffprobe"),
            },
            publisher: CopyPublisher,
        };
        let request = Request {
            operation: "tools.collage".to_string(),
            inputs: vec![p1.clone()],
            ..Default::default()
        };

        let outcome = tools.run(&request, &Selection::inspect(&[p1]));
        assert!(matches!(outcome, Outcome::Failed { .. }));

        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    fn annotate_operation_draws_on_photo() {
        let scratch = std::env::temp_dir().join(format!("ot-annotate-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&scratch);

        let p1 = scratch.join("sample.png");
        let img1 = image::RgbaImage::from_pixel(200, 200, image::Rgba([240, 240, 240, 255]));
        img1.save(&p1).unwrap();

        let tools = Tools {
            ffmpeg_tools: crate::media::locator::Tools {
                ffmpeg: PathBuf::from("ffmpeg"),
                ffprobe: PathBuf::from("ffprobe"),
            },
            publisher: CopyPublisher,
        };
        let request = Request {
            operation: "tools.annotate".to_string(),
            inputs: vec![p1.clone()],
            settings: ToolSettings {
                annotations_data: vec![
                    crate::media::annotate::Annotation::Arrow {
                        x0: 0.2,
                        y0: 0.2,
                        x1: 0.8,
                        y1: 0.8,
                        color: [255, 92, 0, 255],
                        width: 4.0,
                    },
                    crate::media::annotate::Annotation::Text {
                        x: 0.1,
                        y: 0.1,
                        text: "LABEL".to_string(),
                        color: [0, 0, 0, 255],
                        scale: 2,
                    },
                ],
                ..Default::default()
            },
        };

        let outcome = tools.run(&request, &Selection::inspect(&[p1.clone()]));
        let outputs = match outcome {
            Outcome::Succeeded { outputs } => outputs,
            Outcome::Failed { reason } => panic!("annotate failed: {:?}", reason),
            Outcome::Cancelled => panic!("cancelled"),
        };
        assert_eq!(outputs.len(), 1);
        let out_file = &outputs[0];
        assert!(out_file.is_file(), "output exists: {:?}", out_file);
        assert!(out_file.to_string_lossy().contains("sample annotated.png"));

        let annotated_img = image::open(out_file).expect("annotated opens");
        assert_eq!(annotated_img.width(), 200);
        assert_eq!(annotated_img.height(), 200);

        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    fn edit_image_operation_applies_adjustments() {
        let scratch = std::env::temp_dir().join(format!("ot-edit-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&scratch);

        let p1 = scratch.join("sample.jpg");
        let img1 = image::RgbImage::from_pixel(100, 100, image::Rgb([120, 120, 120]));
        img1.save(&p1).unwrap();

        let tools = Tools {
            ffmpeg_tools: crate::media::locator::Tools {
                ffmpeg: PathBuf::from("ffmpeg"),
                ffprobe: PathBuf::from("ffprobe"),
            },
            publisher: CopyPublisher,
        };
        let request = Request {
            operation: "tools.edit_image".to_string(),
            inputs: vec![p1.clone()],
            settings: ToolSettings {
                photo_adjustments: Some(crate::media::adjustments::PhotoAdjustments {
                    brightness_x100: 25,
                    contrast_x100: 120,
                    vignette_x100: 50,
                    ..Default::default()
                }),
                ..Default::default()
            },
        };

        let outcome = tools.run(&request, &Selection::inspect(&[p1.clone()]));
        let outputs = match outcome {
            Outcome::Succeeded { outputs } => outputs,
            Outcome::Failed { reason } => panic!("adjust failed: {:?}", reason),
            Outcome::Cancelled => panic!("cancelled"),
        };
        assert_eq!(outputs.len(), 1);
        let out_file = &outputs[0];
        assert!(out_file.is_file(), "output exists: {:?}", out_file);
        assert!(out_file.to_string_lossy().contains("sample adjusted.jpg"));

        let adjusted_img = image::open(out_file).expect("adjusted opens");
        assert_eq!(adjusted_img.width(), 100);
        assert_eq!(adjusted_img.height(), 100);

        let _ = std::fs::remove_dir_all(&scratch);
    }

    /// A staging name belongs to one run, and only one: two names taken
    /// one after another are never the same, and each carries the
    /// program's own id, so another process going at the same file
    /// cannot say the name either.
    #[test]
    fn a_staging_name_is_one_runs_own() {
        let file = PathBuf::from("C:\\somewhere").join("clip.mp4");
        let first = staging_name(&file, "mp4.part");
        let second = staging_name(&file, "mp4.part");
        assert_ne!(first, second, "no two runs say one name");
        assert!(first != file, "the staging name is not the file's name");
        assert!(
            first.to_string_lossy().contains(&format!(".mp4.part")),
            "the kind of work is the last word said"
        );
        assert!(
            first.to_string_lossy().contains(&std::process::id().to_string()),
            "the program's own id is said in the name"
        );
    }

    /// A run that stops anywhere short of the publish takes its
    /// half-made work back with it: the user's folder keeps nothing of
    /// ours.
    #[test]
    fn a_run_that_cannot_publish_sweeps_its_work() {
        struct StandingPublisher;
        impl crate::media::publish::Publisher for StandingPublisher {
            fn publish(
                &self,
                _temporary: &Path,
                _destination: &Path,
            ) -> Result<(), crate::media::publish::PublishError> {
                Err(crate::media::publish::PublishError::CouldNotMove)
            }
        }
        let scratch = std::env::temp_dir().join("open-tangerine-sweep-test");
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(&scratch).expect("the scratch folder was made");
        let input = scratch.join("photo.png");
        // A picture the picture library can read and cover, and a
        // publisher that can never put it where it belongs: the run
        // stops at the last page, and the sweep takes the staging back.
        image::DynamicImage::new_rgb8(64, 48)
            .save(&input)
            .expect("the test picture was made");

        let tools = Tools {
            ffmpeg_tools: crate::media::locator::Tools {
                ffmpeg: PathBuf::new(),
                ffprobe: PathBuf::new(),
            },
            publisher: StandingPublisher,
        };
        let request = Request {
            operation: "tools.redact_image".to_string(),
            inputs: vec![input.clone()],
            settings: ToolSettings {
                redact_areas: vec![RedactArea {
                    x_mille: 0,
                    y_mille: 0,
                    w_mille: 300,
                    h_mille: 300,
                    start_ms: None,
                    end_ms: None,
                }],
                ..Default::default()
            },
            ..Default::default()
        };
        let outcome = tools.run(&request, &Selection::inspect(&[input.clone()]));
        assert!(
            matches!(outcome, Outcome::Failed { .. }),
            "the publish could not happen: {outcome:?}"
        );
        let leftovers: Vec<PathBuf> = std::fs::read_dir(&scratch)
            .expect("the folder is there")
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .map(|name| name.to_string_lossy().contains("redact"))
                    .unwrap_or(false)
            })
            .collect();
        assert!(
            leftovers.is_empty(),
            "nothing of the run was left beside the file, yet {} stood",
            leftovers.first().map(|p| p.display().to_string()).unwrap_or_default()
        );
        let _ = std::fs::remove_dir_all(&scratch);
    }

    /// Two runs of the same file, going at once, pour into two buckets:
    /// with one staging name of each run's own, both conversions finish
    /// and both results stand.
    #[test]
    #[ignore = "needs the conversion program"]
    fn two_runs_of_one_file_stand_beside_each_other() {
        let ffmpeg = Path::new("vendor").join("ffmpeg").join("ffmpeg.exe");
        let ffprobe = Path::new("vendor").join("ffmpeg").join("ffprobe.exe");
        if !ffmpeg.is_file() || !ffprobe.is_file() {
            eprintln!("the program is not there; this test needs it");
            return;
        }
        struct MovePublisher;
        impl crate::media::publish::Publisher for MovePublisher {
            fn publish(
                &self,
                temporary: &Path,
                destination: &Path,
            ) -> Result<(), crate::media::publish::PublishError> {
                if destination.is_file() {
                    return Err(crate::media::publish::PublishError::DestinationExists);
                }
                std::fs::rename(temporary, destination)
                    .map(|_| ())
                    .map_err(|_| crate::media::publish::PublishError::CouldNotMove)
            }
        }
        let scratch = std::env::temp_dir()
            .join("open-tangerine-staging-race")
            .join(std::process::id().to_string());
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(&scratch).expect("the scratch folder was made");
        let input = make_test_video(&scratch);
        let where_the_program_is = crate::media::locator::Tools {
            ffmpeg: ffmpeg.clone(),
            ffprobe: ffprobe.clone(),
        };

        let runs = [
            (0u64, 1_000u64),
            (1_000, 2_000),
        ]
        .map(|(start, end)| {
            let tools = Tools {
                ffmpeg_tools: where_the_program_is.clone(),
                publisher: MovePublisher,
            };
            let input = input.clone();
            std::thread::spawn(move || {
                tools.run(
                    &Request {
                        operation: "tools.trim".to_string(),
                        inputs: vec![input.clone()],
                        settings: ToolSettings {
                            start_ms: Some(start),
                            end_ms: Some(end),
                            ..Default::default()
                        },
                        ..Default::default()
                    },
                    &Selection::inspect(std::slice::from_ref(&input)),
                )
            })
            .join()
            .expect("the run came home")
        });
        let outputs = |outcome: Outcome| match outcome {
            Outcome::Succeeded { outputs } => outputs,
            other => panic!("a run failed: {other:?}"),
        };
        let a = outputs(runs[0].clone());
        let b = outputs(runs[1].clone());
        assert_eq!(a.len(), 1, "one file out of the first run");
        assert_eq!(b.len(), 1, "one file out of the second run");
        assert_ne!(a[0], b[0], "the two runs' results are two files");
        let held_a = crate::media::frames::duration_of(&where_the_program_is, &a[0])
            .expect("the first result has a length");
        let held_b = crate::media::frames::duration_of(&where_the_program_is, &b[0])
            .expect("the second result has a length");
        assert!(
            (held_a - 1.0).abs() < 0.35 && (held_b - 1.0).abs() < 0.35,
            "both results carry near a second, said {}s and {}s",
            held_a,
            held_b
        );
        let _ = std::fs::remove_dir_all(&scratch);
    }
