//! What the wheel can offer.
//!
//! The definitions live here, away from the renderer, so a later milestone can
//! fill them in from the real conversion matrix or the real tool list without
//! touching the drawing code.
//!
//! The operations themselves are in the registry, which is where the rules
//! about what may be used for a selection live. This module holds the
//! vocabulary the rest of the program speaks: the wheels, the modifier keys,
//! and what a choice looks like.

use super::registry::{Operation, operations_for};

/// The three wheels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RadialMode {
    Convert,
    Tools,
    Actions,
}

impl RadialMode {
    /// Every mode, in the order the wheel steps through them.
    pub const ALL: [RadialMode; 3] = [RadialMode::Convert, RadialMode::Tools, RadialMode::Actions];

    /// The heading shown in the hub.
    pub fn title(self) -> &'static str {
        match self {
            RadialMode::Convert => "CONVERT",
            RadialMode::Tools => "TOOLS",
            RadialMode::Actions => "ACTIONS",
        }
    }

    /// The name used in diagnostics and in the accessibility label.
    pub fn name(self) -> &'static str {
        match self {
            RadialMode::Convert => "convert",
            RadialMode::Tools => "tools",
            RadialMode::Actions => "file actions",
        }
    }

    /// The next mode, wrapping round. A negative step goes the other way.
    pub fn stepped(self, delta: i32) -> RadialMode {
        let count = RadialMode::ALL.len() as i32;
        let index = (self as i32 + delta).rem_euclid(count);
        RadialMode::ALL[index as usize]
    }
}

/// One of the modifier keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Modifier {
    Shift,
    Ctrl,
    Alt,
}

/// Which modifier opens which wheel.
///
/// The mapping is configuration rather than a rule baked into the code. On
/// Windows `Ctrl` and `Alt` already mean "copy" and "link" inside Explorer's
/// own drag, so Milestone 1 leaves those two wheels to the scroll wheel and
/// only claims `Shift`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModifierMap {
    /// The modifier that opens the convert wheel.
    pub convert: Option<Modifier>,
    /// The modifier that opens the tools wheel.
    pub tools: Option<Modifier>,
    /// The modifier that opens the file actions wheel.
    pub actions: Option<Modifier>,
}

impl Default for ModifierMap {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl ModifierMap {
    /// The mapping Milestone 1 runs with.
    pub const DEFAULT: ModifierMap = ModifierMap {
        convert: Some(Modifier::Shift),
        tools: None,
        actions: None,
    };

    /// The mapping the macOS product uses, kept for reference.
    pub const MACOS: ModifierMap = ModifierMap {
        convert: Some(Modifier::Shift),
        tools: Some(Modifier::Ctrl),
        actions: Some(Modifier::Alt),
    };

    /// The wheel the held modifiers ask for.
    pub fn mode_for(&self, modifiers: Modifiers) -> Option<RadialMode> {
        let held = |wanted: Option<Modifier>| -> bool {
            match wanted {
                Some(Modifier::Shift) => modifiers.shift,
                Some(Modifier::Ctrl) => modifiers.ctrl,
                Some(Modifier::Alt) => modifiers.alt,
                None => false,
            }
        };
        if held(self.actions) {
            Some(RadialMode::Actions)
        } else if held(self.tools) {
            Some(RadialMode::Tools)
        } else if held(self.convert) {
            Some(RadialMode::Convert)
        } else {
            None
        }
    }
}

/// The modifier keys, as the platform layer sees them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
}

impl Modifiers {
    pub const NONE: Modifiers = Modifiers {
        shift: false,
        ctrl: false,
        alt: false,
    };

    /// Whether any modifier at all is held.
    pub fn any(self) -> bool {
        self.shift || self.ctrl || self.alt
    }
}

/// The picture a segment shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IconKind {
    Picture,
    Document,
    Film,
    Archive,
    Folder,
    Lines,
    Copy,
    Pencil,
    Merge,
    Scissors,
    Gauge,
}

/// One choice on the wheel.
#[derive(Clone, Debug, PartialEq)]
pub struct Segment {
    /// A stable identifier, used in diagnostics and by later milestones.
    pub id: String,
    /// The short name drawn on the segment.
    pub label: String,
    /// The longer line shown while the segment is highlighted.
    pub detail: String,
    /// The picture drawn on the segment.
    pub icon: IconKind,
}

impl Segment {
    /// Makes a choice for the wheel.
    pub fn new(id: &str, label: &str, detail: &str, icon: IconKind) -> Self {
        Self {
            id: id.to_string(),
            label: label.to_string(),
            detail: detail.to_string(),
            icon,
        }
    }
}

/// The choices each wheel offers.
///
/// The operations themselves live in the registry, which also knows which of
/// them can really be used for the files that were dragged. The wheel shows
/// every operation of the wheel for now: whether one may be offered is the
/// registry's business, and the menu does not depend on the selection yet.
pub fn segments_for(mode: RadialMode) -> Vec<Segment> {
    operations_for(mode).map(Operation::as_segment).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What a segment looks like, in one line.
    fn shape(segment: &Segment) -> String {
        format!(
            "{} | {} | {} | {:?}",
            segment.id, segment.label, segment.detail, segment.icon
        )
    }

    #[test]
    fn every_mode_has_segments() {
        for mode in RadialMode::ALL {
            assert!(
                !segments_for(mode).is_empty(),
                "{} has no segments",
                mode.name()
            );
        }
    }

    #[test]
    fn the_menu_shows_exactly_what_it_showed_before() {
        // The operations come from the registry now, but the menu must look
        // exactly the way it did: the same choices, in the same order, with
        // the same names and the same pictures. This is what pins it.
        let expected: Vec<(&str, Vec<&str>)> = vec![
            (
                "convert",
                vec![
                    "convert.png | PNG | Convert to PNG | Picture",
                    "convert.jpeg | JPEG | Convert to JPEG | Picture",
                    "convert.webp | WebP | Convert to WebP | Picture",
                    "convert.pdf | PDF | Convert to PDF | Document",
                    "convert.mp4 | MP4 | Convert to MP4 | Film",
                    "convert.gif | GIF | Convert to GIF | Film",
                    "convert.tiff | TIFF | Convert to TIFF | Picture",
                    "convert.bmp | BMP | Convert to BMP | Picture",
                ],
            ),
            (
                "tools",
                vec![
                    "tools.compress | Compress | Make the file smaller | Gauge",
                    "tools.crop | Crop | Trim the edges away | Scissors",
                    "tools.trim | Trim | Cut the beginning or the end | Scissors",
                    "tools.merge | Merge | Join several files together | Merge",
                    "tools.metadata | Metadata | Edit the file metadata | Pencil",
                ],
            ),
            (
                "actions",
                vec![
                    "actions.copy-path | Path | Copy the full path | Lines",
                    "actions.duplicate | Copy | Make a duplicate | Copy",
                    "actions.rename | Rename | Give the file a new name | Pencil",
                    "actions.zip | ZIP | Put the files in an archive | Archive",
                    "actions.reveal | Show | Reveal in Explorer | Folder",
                ],
            ),
        ];
        let modes = [
            ("convert", RadialMode::Convert),
            ("tools", RadialMode::Tools),
            ("actions", RadialMode::Actions),
        ];
        for ((name, wanted), (_, mode)) in expected.iter().zip(modes) {
            let got: Vec<String> = segments_for(mode).iter().map(shape).collect();
            assert_eq!(&got, wanted, "the {} wheel has changed", name);
        }
    }

    #[test]
    fn segment_ids_are_unique_inside_a_mode() {
        for mode in RadialMode::ALL {
            let segments = segments_for(mode);
            let mut ids: Vec<&str> = segments.iter().map(|s| s.id.as_str()).collect();
            ids.sort_unstable();
            let count = ids.len();
            ids.dedup();
            assert_eq!(ids.len(), count, "{} has duplicate ids", mode.name());
        }
    }

    #[test]
    fn stepping_wraps_in_both_directions() {
        assert_eq!(RadialMode::Convert.stepped(1), RadialMode::Tools);
        assert_eq!(RadialMode::Actions.stepped(1), RadialMode::Convert);
        assert_eq!(RadialMode::Convert.stepped(-1), RadialMode::Actions);
        assert_eq!(RadialMode::Tools.stepped(0), RadialMode::Tools);
    }

    #[test]
    fn the_windows_modifier_mapping_picks_the_right_wheel() {
        let map = ModifierMap::MACOS;
        let shift = Modifiers {
            shift: true,
            ..Modifiers::NONE
        };
        let ctrl = Modifiers {
            ctrl: true,
            ..Modifiers::NONE
        };
        let alt = Modifiers {
            alt: true,
            ..Modifiers::NONE
        };
        assert_eq!(map.mode_for(shift), Some(RadialMode::Convert));
        assert_eq!(map.mode_for(ctrl), Some(RadialMode::Tools));
        assert_eq!(map.mode_for(alt), Some(RadialMode::Actions));
        assert_eq!(map.mode_for(Modifiers::NONE), None);
    }

    #[test]
    fn a_stronger_modifier_wins() {
        let map = ModifierMap::MACOS;
        let both = Modifiers {
            shift: true,
            ctrl: true,
            ..Modifiers::NONE
        };
        assert_eq!(map.mode_for(both), Some(RadialMode::Tools));
    }

    #[test]
    fn the_default_mapping_only_claims_shift() {
        let map = ModifierMap::DEFAULT;
        let shift = Modifiers {
            shift: true,
            ..Modifiers::NONE
        };
        assert_eq!(map.mode_for(shift), Some(RadialMode::Convert));
        assert_eq!(
            map.mode_for(Modifiers {
                ctrl: true,
                ..Modifiers::NONE
            }),
            None
        );
        assert_eq!(
            map.mode_for(Modifiers {
                alt: true,
                ..Modifiers::NONE
            }),
            None
        );
    }
}
