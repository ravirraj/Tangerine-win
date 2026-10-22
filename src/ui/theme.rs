//! The colours of the wheel.
//!
//! The wheel is drawn in one theme or the other: a light one and a dark one.
//! The tokens are the same in both, so that a drawing only has to ask for
//! "the colour of a choice that is not pointed at" and never for a colour of
//! its own.
//!
//! The themes follow the appearance Windows is asking for, the way the macOS
//! program follows the appearance of the system. A person who has a light
//! system gets a light wheel.

/// A colour, as the four numbers the renderer wants: red, green, blue and
/// alpha.
pub type Rgba = (u8, u8, u8, u8);

/// The colours the wheel is drawn with.
///
/// Every token is used somewhere: the background is behind the wheel, the
/// surface is the hub in the middle, the segment is a choice that is not
/// pointed at, the hover and the pressed are a choice that is being pointed
/// at and one that is being pressed, and the accent is the choice that has
/// been made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Theme {
    /// The colour behind the wheel.
    pub background: Rgba,
    /// The colour of the hub in the middle.
    pub surface: Rgba,
    /// The colour of a choice that is not pointed at.
    pub segment: Rgba,
    /// The colour of a choice the pointer is on.
    pub hover: Rgba,
    /// The colour of a choice that is being pressed.
    pub pressed: Rgba,
    /// The colour of the choice that has been made.
    pub accent: Rgba,
    /// The colour of the choice that has been made, and the pointer is on it.
    pub accent_hover: Rgba,
    /// The colour of the choice that has been made, and it is being pressed.
    pub accent_pressed: Rgba,
    /// The colour of the words.
    pub text: Rgba,
    /// The colour of the words that are not the point.
    pub text_secondary: Rgba,
    /// The colour of the line between two choices.
    pub border: Rgba,
}

impl Theme {
    /// The theme for a light appearance.
    pub fn light() -> Self {
        Self {
            background: (0xf7, 0xf7, 0xf5, 0xff),
            surface: (0xff, 0xff, 0xff, 0xff),
            segment: (0xec, 0xed, 0xef, 0xff),
            hover: (0xe3, 0xe5, 0xeb, 0xff),
            pressed: (0xd8, 0xdb, 0xdf, 0xff),
            accent: (0xff, 0x5a, 0x1f, 0xff),
            accent_hover: (0xff, 0x71, 0x38, 0xff),
            accent_pressed: (0xe9, 0x4a, 0x12, 0xff),
            text: (0x24, 0x24, 0x24, 0xff),
            text_secondary: (0x73, 0x73, 0x73, 0xff),
            border: (0xe0, 0xe2, 0xe5, 0xff),
        }
    }

    /// The theme for a dark appearance.
    pub fn dark() -> Self {
        Self {
            background: (0x18, 0x18, 0x18, 0xff),
            surface: (0x25, 0x25, 0x25, 0xff),
            segment: (0x30, 0x32, 0x36, 0xff),
            hover: (0x3a, 0x3d, 0x42, 0xff),
            pressed: (0x45, 0x49, 0x50, 0xff),
            accent: (0xff, 0x5a, 0x1f, 0xff),
            accent_hover: (0xff, 0x71, 0x38, 0xff),
            accent_pressed: (0xe9, 0x4a, 0x12, 0xff),
            text: (0xf5, 0xf5, 0xf5, 0xff),
            text_secondary: (0xb3, 0xb3, 0xb3, 0xff),
            border: (0x41, 0x43, 0x48, 0xff),
        }
    }

    /// The theme for an appearance.
    pub fn for_appearance(dark: bool) -> Self {
        if dark { Self::dark() } else { Self::light() }
    }

    /// The colour of the words on a choice that has been made.
    ///
    /// The accent is dark enough in both themes for the words on it to be
    /// white, and white is what the design shows.
    pub fn accent_text(&self) -> Rgba {
        (0xff, 0xff, 0xff, 0xff)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_two_themes_have_the_same_tokens() {
        // The themes are two sets of the same names, so a drawing that asks
        // for a token always gets one.
        let light = Theme::light();
        let dark = Theme::dark();
        assert_eq!(light.accent, dark.accent);
        assert_eq!(light.accent_hover, dark.accent_hover);
        assert_eq!(light.accent_pressed, dark.accent_pressed);
        assert_ne!(light.background, dark.background);
        assert_ne!(light.text, dark.text);
    }

    #[test]
    fn the_accent_is_the_designs_orange() {
        for theme in [Theme::light(), Theme::dark()] {
            assert_eq!(theme.accent, (0xff, 0x5a, 0x1f, 0xff));
            assert_eq!(theme.accent_hover, (0xff, 0x71, 0x38, 0xff));
            assert_eq!(theme.accent_pressed, (0xe9, 0x4a, 0x12, 0xff));
        }
    }

    #[test]
    fn the_light_theme_is_the_designs() {
        let theme = Theme::light();
        assert_eq!(theme.background, (0xf7, 0xf7, 0xf5, 0xff));
        assert_eq!(theme.surface, (0xff, 0xff, 0xff, 0xff));
        assert_eq!(theme.segment, (0xec, 0xed, 0xef, 0xff));
        assert_eq!(theme.hover, (0xe3, 0xe5, 0xeb, 0xff));
        assert_eq!(theme.pressed, (0xd8, 0xdb, 0xdf, 0xff));
        assert_eq!(theme.text, (0x24, 0x24, 0x24, 0xff));
        assert_eq!(theme.text_secondary, (0x73, 0x73, 0x73, 0xff));
        assert_eq!(theme.border, (0xe0, 0xe2, 0xe5, 0xff));
    }

    #[test]
    fn the_dark_theme_is_the_designs() {
        let theme = Theme::dark();
        assert_eq!(theme.background, (0x18, 0x18, 0x18, 0xff));
        assert_eq!(theme.surface, (0x25, 0x25, 0x25, 0xff));
        assert_eq!(theme.segment, (0x30, 0x32, 0x36, 0xff));
        assert_eq!(theme.hover, (0x3a, 0x3d, 0x42, 0xff));
        assert_eq!(theme.pressed, (0x45, 0x49, 0x50, 0xff));
        assert_eq!(theme.text, (0xf5, 0xf5, 0xf5, 0xff));
        assert_eq!(theme.text_secondary, (0xb3, 0xb3, 0xb3, 0xff));
        assert_eq!(theme.border, (0x41, 0x43, 0x48, 0xff));
    }

    #[test]
    fn the_words_on_a_choice_that_was_made_are_white() {
        for theme in [Theme::light(), Theme::dark()] {
            assert_eq!(theme.accent_text(), (0xff, 0xff, 0xff, 0xff));
        }
    }

    #[test]
    fn the_appearance_picks_the_theme() {
        assert_eq!(Theme::for_appearance(false), Theme::light());
        assert_eq!(Theme::for_appearance(true), Theme::dark());
    }
}
