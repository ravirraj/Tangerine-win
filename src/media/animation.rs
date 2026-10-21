//! Deciding whether a file holds more than one frame.
//!
//! A conversion writes one picture. A file that holds several would lose
//! all but the first of them, so it is not converted at all: the file is
//! refused and the user is told why.
//!
//! The count comes from the frames the program really decoded, and a file
//! whose count cannot be established is refused as well. Nothing is
//! guessed, because a guess that is wrong loses a picture without saying
//! so.

use std::path::Path;

use crate::media::command::count_frames;
use crate::media::manifest::FormatSupport;
use crate::media::runner::ProcessRunner;

/// What the count of frames came to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Count {
    /// The frames that were decoded.
    Frames(u64),
    /// The program would not read the file with that demuxer.
    Refused,
    /// The program read the file but would not say how many frames it has.
    Unknown,
}

/// What a file turned out to be.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// One picture. It can be converted.
    Still,
    /// More than one picture. It is an animation, and this phase does not
    /// convert animations.
    Animated,
    /// It could not be told. It is not converted, because it might be an
    /// animation and nothing would say so.
    Uncertain,
}

impl Verdict {
    /// Whether the file may be converted.
    pub fn may_convert(self) -> bool {
        matches!(self, Verdict::Still)
    }
}

/// Counts the frames of a file, with the demuxer that can see all of them.
///
/// The demuxer is named, so that a file whose name holds a percent sign is
/// not taken for a picture sequence.
pub fn count(runner: &dyn ProcessRunner, program: &Path, demuxer: &str, input: &Path) -> Count {
    let args = count_frames(input, demuxer);
    let result = match runner.run(program, &args, None) {
        Ok(result) => result,
        Err(_) => return Count::Refused,
    };
    if !result.succeeded() {
        return Count::Refused;
    }
    let text = String::from_utf8_lossy(&result.stdout);
    let answer = text.trim();
    match answer.parse::<u64>() {
        // A count of zero is not a count: nothing was decoded, which is not
        // the same as a file with no frames in it.
        Ok(frames) if frames > 0 => Count::Frames(frames),
        _ => Count::Unknown,
    }
}

/// Says what a file is, so that it can be converted or refused.
///
/// There are two kinds of format here. One has a demuxer that reads still
/// files as well as animated ones, and the count it gives is the answer.
/// The other has a demuxer that reads only animations, and a file it
/// refuses may be a still one or may be broken; the still one is then
/// confirmed by being read as a single picture, and a file that cannot be
/// confirmed is refused.
pub fn judge(
    runner: &dyn ProcessRunner,
    program: &Path,
    format: &FormatSupport,
    input: &Path,
) -> Verdict {
    let Some(animation) = &format.animation else {
        // The format cannot be animated, so there is nothing to check.
        return Verdict::Still;
    };

    match count(runner, program, &animation.demuxer, input) {
        Count::Frames(frames) => {
            // The demuxer that reads every frame read this file, so the file
            // is one of the kind it reads. Where it reads only animations,
            // that settles the matter: the file is an animation, whatever
            // the number of frames it holds.
            if format.confirm.is_some() {
                Verdict::Animated
            } else if frames == 1 {
                Verdict::Still
            } else {
                Verdict::Animated
            }
        }
        Count::Refused | Count::Unknown => {
            // The file was not read as an animation. That is not proof that
            // it is a still picture, so it has to be confirmed.
            let Some(confirm) = &format.confirm else {
                // There is nothing to confirm it with, so it cannot be
                // converted.
                return Verdict::Uncertain;
            };
            match count(runner, program, &confirm.demuxer, input) {
                Count::Frames(1) => Verdict::Still,
                Count::Frames(_) => Verdict::Animated,
                _ => Verdict::Uncertain,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::manifest::Manifest;
    use crate::media::runner::RunResult;
    use std::ffi::OsString;
    use std::path::PathBuf;

    /// A runner that answers with a number of frames, or refuses.
    struct Scripted {
        frames: Option<u64>,
        refuses: bool,
    }

    impl ProcessRunner for Scripted {
        fn run(
            &self,
            _program: &Path,
            _args: &[OsString],
            _stdout_to: Option<&std::fs::File>,
        ) -> std::io::Result<RunResult> {
            if self.refuses {
                return Ok(RunResult {
                    status: Some(1),
                    stdout: Vec::new(),
                    stderr: String::new(),
                });
            }
            Ok(RunResult {
                status: Some(0),
                stdout: format!("{}\n", self.frames.unwrap_or(0)).into_bytes(),
                stderr: String::new(),
            })
        }
    }

    /// A runner that answers differently for the two demuxers.
    struct TwoAnswers {
        animation: Option<u64>,
        confirm: Option<u64>,
    }

    impl ProcessRunner for TwoAnswers {
        fn run(
            &self,
            _program: &Path,
            args: &[OsString],
            _stdout_to: Option<&std::fs::File>,
        ) -> std::io::Result<RunResult> {
            let demuxer = args
                .iter()
                .position(|arg| arg == "-f")
                .and_then(|at| args.get(at + 1))
                .map(|arg| arg.to_string_lossy().into_owned())
                .unwrap_or_default();
            let frames = if demuxer == "webp_anim" {
                self.animation
            } else {
                self.confirm
            };
            match frames {
                Some(frames) => Ok(RunResult {
                    status: Some(0),
                    stdout: format!("{frames}\n").into_bytes(),
                    stderr: String::new(),
                }),
                None => Ok(RunResult {
                    status: Some(1),
                    stdout: Vec::new(),
                    stderr: String::new(),
                }),
            }
        }
    }

    fn program() -> PathBuf {
        PathBuf::from("ffprobe")
    }

    #[test]
    fn a_format_that_cannot_be_animated_is_a_still_picture() {
        let manifest = Manifest::get();
        let jpeg = manifest.format("jpeg").expect("jpeg is in the manifest");
        let runner = Scripted {
            frames: None,
            refuses: true,
        };
        assert_eq!(
            judge(&runner, &program(), jpeg, Path::new("photo.jpeg")),
            Verdict::Still
        );
    }

    #[test]
    fn one_frame_is_a_still_picture() {
        let manifest = Manifest::get();
        let gif = manifest.format("gif").expect("gif is in the manifest");
        let runner = Scripted {
            frames: Some(1),
            refuses: false,
        };
        assert_eq!(
            judge(&runner, &program(), gif, Path::new("photo.gif")),
            Verdict::Still
        );
    }

    #[test]
    fn several_frames_are_an_animation() {
        let manifest = Manifest::get();
        let gif = manifest.format("gif").expect("gif is in the manifest");
        let runner = Scripted {
            frames: Some(4),
            refuses: false,
        };
        assert_eq!(
            judge(&runner, &program(), gif, Path::new("photo.gif")),
            Verdict::Animated
        );
    }

    #[test]
    fn a_count_that_is_not_known_is_not_a_still_picture() {
        let manifest = Manifest::get();
        let gif = manifest.format("gif").expect("gif is in the manifest");
        let runner = Scripted {
            frames: None,
            refuses: false,
        };
        assert_eq!(
            judge(&runner, &program(), gif, Path::new("photo.gif")),
            Verdict::Uncertain
        );
    }

    #[test]
    fn a_webp_that_is_read_as_an_animation_is_an_animation() {
        let manifest = Manifest::get();
        let webp = manifest.format("webp").expect("webp is in the manifest");
        let runner = TwoAnswers {
            animation: Some(2),
            confirm: Some(1),
        };
        assert_eq!(
            judge(&runner, &program(), webp, Path::new("photo.webp")),
            Verdict::Animated
        );
    }

    #[test]
    fn a_webp_that_is_refused_as_an_animation_is_confirmed_as_a_still_one() {
        let manifest = Manifest::get();
        let webp = manifest.format("webp").expect("webp is in the manifest");
        let runner = TwoAnswers {
            animation: None,
            confirm: Some(1),
        };
        assert_eq!(
            judge(&runner, &program(), webp, Path::new("photo.webp")),
            Verdict::Still
        );
    }

    #[test]
    fn a_webp_that_cannot_be_confirmed_is_not_converted() {
        let manifest = Manifest::get();
        let webp = manifest.format("webp").expect("webp is in the manifest");
        let runner = TwoAnswers {
            animation: None,
            confirm: None,
        };
        assert_eq!(
            judge(&runner, &program(), webp, Path::new("photo.webp")),
            Verdict::Uncertain
        );
        assert!(!Verdict::Uncertain.may_convert());
    }
}
