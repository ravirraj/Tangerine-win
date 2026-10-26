//! The wheel as it is drawn.
//!
//! The Slint component only ever receives plain values: every position, size
//! and colour is worked out here, in Rust, where it can be tested.

use std::rc::Rc;

use slint::platform::software_renderer::{
    MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType, TargetPixel,
};
use slint::platform::{Platform, PlatformError, WindowAdapter};
use slint::{ModelRc, PhysicalSize, VecModel};

use crate::radial::actions::{RadialMode, Segment};
use crate::radial::geometry::{RadialGeometry, WheelProfile};
use crate::radial::icons;
use crate::ui::theme::{Rgba, Theme};
slint::include_modules!();

/// The width of the wheel in logical pixels, before the display's own scale.
///
/// This is the design's wheel drawn smaller: the design measures it at
/// three hundred and sixty pixels across, and everything in it — the radii,
/// the button in the middle, the pictures and the names — is drawn by the
/// same factor, so that the wheel looks the same as the design.
pub const DISC_SIZE: f32 = WheelProfile::design().size;

/// The size of the design's wheel, in logical pixels.
pub const DESIGN_SIZE: f32 = crate::radial::geometry::DESIGN_SIZE;

/// The diameter of the button in the middle, in logical pixels.
///
/// The design's centre button is a hundred and four across, and the hub of
/// the wheel is sixty: the button stands inside the hub, with a ring of
/// eight around it.
pub fn hub_size() -> f32 {
    104.0 * DISC_SIZE / DESIGN_SIZE
}

/// The size of the picture on a choice, in logical pixels.
fn icon_size() -> f32 {
    22.0 * DISC_SIZE / DESIGN_SIZE
}

/// The size of the name of a choice, in logical pixels.
fn label_size() -> f32 {
    13.0 * DISC_SIZE / DESIGN_SIZE
}

/// The size of the name in the middle of the wheel, in logical pixels.
fn centre_label_size() -> f32 {
    16.0 * DISC_SIZE / DESIGN_SIZE
}

/// A pixel in the order `UpdateLayeredWindow` wants: blue, green, red, alpha.
#[repr(transparent)]
#[derive(Clone, Copy, Default)]
pub struct Bgra8(u32);

impl Bgra8 {
    pub(crate) fn parts(self) -> (u8, u8, u8, u8) {
        (
            (self.0 >> 24) as u8,
            (self.0 >> 16) as u8,
            (self.0 >> 8) as u8,
            self.0 as u8,
        )
    }
}

impl TargetPixel for Bgra8 {
    fn blend(&mut self, color: PremultipliedRgbaColor) {
        let (a, r, g, b) = self.parts();
        let inverse = (u8::MAX - color.alpha) as u16;
        let blue = (b as u16 * inverse / 255) as u32 + color.blue as u32;
        let green = (g as u16 * inverse / 255) as u32 + color.green as u32;
        let red = (r as u16 * inverse / 255) as u32 + color.red as u32;
        let alpha = a as u16 + color.alpha as u16 - (a as u16 * color.alpha as u16) / 255;
        self.0 =
            (alpha as u32) << 24 | (red.min(255) << 16) | (green.min(255) << 8) | blue.min(255);
    }

    fn from_rgb(red: u8, green: u8, blue: u8) -> Self {
        Self(0xff00_0000 | (red as u32) << 16 | (green as u32) << 8 | blue as u32)
    }

    fn background() -> Self {
        Self(0)
    }
}

/// The platform that hands out our own window to Slint.
/// The platform of the program: every component is given a window of its
/// own, drawn in software into a buffer the caller holds.
///
/// A component is made right after its window is put in the line, so the
/// component and the window find each other: `create_window_adapter` hands
/// out the windows in the order they were put there, one per component.
/// A component made with none waiting — a component the program did not
/// put a window aside for — is given a window of its own that nobody
/// renders, which is better than the whole program stopping.
struct WheelPlatform;

impl Platform for WheelPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(WINDOWS
            .with(|queue| queue.borrow_mut().pop_front())
            .unwrap_or_else(|| {
                MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer)
            }))
    }
}

thread_local! {
    static WINDOWS: std::cell::RefCell<std::collections::VecDeque<Rc<MinimalSoftwareWindow>>> =
        std::cell::RefCell::new(std::collections::VecDeque::new());
}

/// Puts a window aside for the component that is made next.
///
/// The window and the component belong together; this is the introduction.
/// Every redraw draws the whole card again: these windows are resized to
/// the media they hold, and a renderer that only redraws the lines it
/// remembers changing would leave whatever fell below the window's first
/// size forever undrawn.
pub fn window_for_next_component() -> Rc<MinimalSoftwareWindow> {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    WINDOWS.with(|queue| queue.borrow_mut().push_back(window.clone()));
    window
}

/// Installs the platform the program draws with.
///
/// Installing only works once per process; a second call quietly keeps
/// the first one, which is what every caller wants.
pub fn install_platform() {
    let _ = slint::platform::set_platform(Box::new(WheelPlatform));
}

/// Everything the wheel needs to draw itself.
#[derive(Clone, Debug)]
pub struct ViewModel {
    pub segments: Vec<SegmentData>,
    pub heading: String,
    pub subtitle: String,
    pub badge: String,
    pub mode_index: i32,
    pub appear: f32,
    pub emphasis_path: String,
    pub emphasis: f32,
    pub fading_path: String,
    pub fading: f32,
    pub disc_size: f32,
    pub hub_size: f32,
    pub heading_size: f32,
    pub subtitle_size: f32,
    /// The message about what came of the last operation.
    pub notice_text: String,
    /// Whether that message is bad news.
    pub notice_failed: bool,
    /// How much of that message is showing.
    pub notice_appear: f32,
    /// The colour behind the wheel.
    pub disc_color: Rgba,
    /// The colour of the hub in the middle.
    pub hub_color: Rgba,
    /// The colour of the line between two choices, and of the hub's edge.
    pub border_color: Rgba,
    /// The colour of the words in the hub.
    pub text_color: Rgba,
    /// The colour of the words in the hub that are not the point.
    pub text_secondary_color: Rgba,
    /// The colour of the choice the pointer is on.
    pub emphasis_color: Rgba,
    /// The colour of the edge of that choice.
    pub emphasis_border: Rgba,
    /// The colour of the choice that is losing the pointer.
    pub fading_color: Rgba,
    /// The colour of the edge of that choice.
    pub fading_border: Rgba,
    /// Whether a file thumbnail should be shown in the hub.
    pub has_thumbnail: bool,
    /// The downscaled thumbnail image for the hub.
    pub thumbnail: Option<slint::Image>,
}

/// How the wheel is drawn at this moment.
#[derive(Clone, Debug)]
pub struct ViewOptions {
    /// How many device pixels there are per logical pixel.
    pub scale: f32,
    /// How far the entrance animation has got, from 0 to 1.
    pub appear: f32,
    /// How much the segment under the pointer has lit up.
    pub emphasis: f32,
    /// How much the segment that is losing the pointer has faded.
    pub fading: f32,
    /// The segment that is being pointed at.
    pub emphasis_index: Option<usize>,
    /// The segment that is losing the pointer.
    pub fading_index: Option<usize>,
    /// The message about what came of the last operation.
    pub notice: Option<Notice>,
    /// How much of that message is showing.
    pub notice_appear: f32,
    /// The colours the wheel is drawn with.
    pub theme: Theme,
    /// Whether the choice the pointer is on is being pressed.
    ///
    /// The pointer is only pressing a choice when the wheel is there to be
    /// clicked, which is when the wheel is interactive. While a file is being
    /// dragged the button is down too, but there the pointer is choosing, not
    /// pressing.
    pub preview_pressed: bool,
    /// Whether a file thumbnail should be shown in the hub.
    pub has_thumbnail: bool,
    /// The downscaled thumbnail image for the hub.
    pub thumbnail: Option<slint::Image>,
}

/// A short message about what came of an operation.
///
/// The message says what happened and nothing else: no name of a file and no
/// path. The log has the detail; this is what the person sees.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    /// The words.
    pub text: String,
    /// Whether it is bad news.
    pub failed: bool,
}

/// Builds the view model from the controller's state.
///
/// `geometry` is the shape of the wheel in physical pixels, the very same one
/// the pointer is tested against. The entrance animation grows the wheel from
/// its centre, and that is the only thing that changes the shape.
pub fn build_view_model(
    segments: &[Segment],
    mode: RadialMode,
    highlighted: Option<usize>,
    file_count: usize,
    development: bool,
    geometry: RadialGeometry,
    options: ViewOptions,
) -> ViewModel {
    let scale = options.scale;
    let appear = options.appear.clamp(0.0, 1.0);
    let emphasis = options.emphasis;
    let fading = options.fading;
    let disc_size = DISC_SIZE * scale;
    let theme = options.theme;
    // The choice the pointer is on is filled with the accent of the theme,
    // and its name and its picture are white. While it is being pressed, the
    // accent of the pressed state is used instead.
    let preview_accent = if options.preview_pressed {
        theme.accent_pressed
    } else {
        theme.accent_hover
    };

    let mut geometry = geometry;
    geometry.inner_radius *= appear;
    geometry.outer_radius *= appear;

    let centre = Point::new(disc_size / 2.0, disc_size / 2.0);
    let mut data = Vec::with_capacity(segments.len());

    for (index, segment) in segments.iter().enumerate() {
        let active = highlighted == Some(index);
        let label_point = geometry.label_point(index);
        let icon_size = icon_size() * scale * appear;
        // The picture sits above the name, in a straight line, so that the
        // pair reads the same way on every segment.
        let icon_centre = Point::new(label_point.x, label_point.y - 15.0 * scale * appear);
        let text_centre = Point::new(label_point.x, label_point.y + 10.0 * scale * appear);

        data.push(SegmentData {
            path: geometry.wedge_path(index).into(),
            fill: color(theme.segment),
            stroke: color(theme.border),
            label: segment.label.clone().into(),
            label_color: color(if active {
                theme.accent_text()
            } else {
                theme.text
            }),
            label_size: label_size() * scale * appear,
            label_x: centre.x + text_centre.x,
            label_y: centre.y + text_centre.y,
            label_width: geometry.label_width() * scale,
            icon: icons::path_for(segment.icon).into(),
            icon_color: color(if active {
                theme.accent_text()
            } else {
                theme.text
            }),
            icon_x: centre.x + icon_centre.x - icon_size / 2.0,
            icon_y: centre.y + icon_centre.y - icon_size / 2.0,
            icon_size,
        });
    }

    let emphasis_index = options.emphasis_index;
    let fading_index = options.fading_index;
    let emphasis_path = emphasis_index
        .and_then(|index| segments.get(index))
        .map(|_| emphasised_path(&geometry, emphasis_index.unwrap(), scale))
        .unwrap_or_default();
    let fading_path = fading_index
        .and_then(|index| segments.get(index))
        .map(|_| emphasised_path(&geometry, fading_index.unwrap(), scale))
        .unwrap_or_default();

    let heading = match highlighted.and_then(|index| segments.get(index)) {
        Some(segment) => segment.label.clone(),
        None => mode.title().to_string(),
    };
    let subtitle = match highlighted.and_then(|index| segments.get(index)) {
        Some(segment) => segment.detail.clone(),
        None => {
            if file_count == 0 {
                "No files".to_string()
            } else if file_count == 1 {
                "1 file".to_string()
            } else {
                format!("{file_count} files")
            }
        }
    };

    ViewModel {
        segments: data,
        heading,
        subtitle,
        badge: if development {
            "DEVELOPMENT MODE".to_string()
        } else {
            String::new()
        },
        mode_index: RadialMode::ALL.iter().position(|m| *m == mode).unwrap_or(0) as i32,
        appear,
        emphasis_path,
        emphasis,
        fading_path,
        fading,
        disc_size,
        hub_size: hub_size() * scale * appear,
        heading_size: centre_label_size() * scale * appear,
        subtitle_size: 10.0 * scale * appear,
        notice_text: options
            .notice
            .as_ref()
            .map(|notice| notice.text.clone())
            .unwrap_or_default(),
        notice_failed: options.notice.as_ref().is_some_and(|notice| notice.failed),
        notice_appear: options.notice_appear,
        disc_color: theme.background,
        hub_color: theme.surface,
        border_color: theme.border,
        text_color: theme.text,
        text_secondary_color: theme.text_secondary,
        // The choice the pointer is on is filled with the accent, in one
        // piece: there is no glow and no shadow over it. The choice that is
        // losing the pointer goes back to being a choice, a little darker
        // while it does.
        emphasis_color: preview_accent,
        emphasis_border: preview_accent,
        fading_color: theme.hover,
        fading_border: theme.border,
        has_thumbnail: options.has_thumbnail,
        thumbnail: options.thumbnail,
    }
}

/// The outline of a segment that is being pointed at.
///
/// The segment is pushed a little further out and its corners are a little
/// rounder, so that it looks as if it lifts to meet the pointer.
fn emphasised_path(geometry: &RadialGeometry, index: usize, scale: f32) -> String {
    let mut lifted = *geometry;
    lifted.inner_radius += 1.5 * scale;
    lifted.outer_radius += 4.0 * scale;
    lifted.wedge_path(index)
}

fn color(rgba: (u8, u8, u8, u8)) -> slint::Color {
    // Slint's colour carries the alpha first.
    slint::Color::from_argb_u8(rgba.3, rgba.0, rgba.1, rgba.2)
}

/// The wheel, and the buffer it is drawn into.
pub struct WheelUi {
    window: Rc<MinimalSoftwareWindow>,
    wheel: Wheel,
    buffer: Vec<Bgra8>,
    width: u32,
    height: u32,
}

impl WheelUi {
    /// Creates the wheel. `scale` is the device scale factor of the display.
    pub fn new(scale: f32) -> Result<Self, String> {
        let window = window_for_next_component();
        install_platform();

        let wheel = Wheel::new().map_err(|error| error.to_string())?;

        let size = (DISC_SIZE * scale).round().max(64.0) as u32;
        window.set_size(PhysicalSize::new(size, size));
        let buffer = vec![Bgra8::default(); (size as usize) * (size as usize)];

        Ok(Self {
            window,
            wheel,
            buffer,
            width: size,
            height: size,
        })
    }

    /// Hands the view model to the Slint component.
    pub fn apply(&self, model: &ViewModel) {
        let items: Vec<SegmentData> = model.segments.clone();
        self.wheel.set_segments(ModelRc::new(VecModel::from(items)));
        self.wheel.set_heading(model.heading.clone().into());
        self.wheel.set_subtitle(model.subtitle.clone().into());
        self.wheel.set_badge(model.badge.clone().into());
        self.wheel.set_mode_index(model.mode_index);
        self.wheel.set_appear(model.appear);
        self.wheel.set_disc_size(model.disc_size);
        self.wheel.set_hub_size(model.hub_size);
        self.wheel.set_heading_size(model.heading_size);
        self.wheel.set_subtitle_size(model.subtitle_size);
        self.wheel
            .set_emphasis_path(model.emphasis_path.clone().into());
        self.wheel.set_emphasis(model.emphasis);
        self.wheel.set_fading_path(model.fading_path.clone().into());
        self.wheel.set_fading(model.fading);
        self.wheel.set_notice_text(model.notice_text.clone().into());
        self.wheel.set_notice_failed(model.notice_failed);
        self.wheel.set_notice_appear(model.notice_appear);
        self.wheel.set_disc_color(color(model.disc_color));
        self.wheel.set_hub_color(color(model.hub_color));
        self.wheel.set_border_color(color(model.border_color));
        self.wheel.set_text_color(color(model.text_color));
        self.wheel
            .set_text_secondary_color(color(model.text_secondary_color));
        self.wheel.set_emphasis_color(color(model.emphasis_color));
        self.wheel.set_emphasis_border(color(model.emphasis_border));
        self.wheel.set_fading_color(color(model.fading_color));
        self.wheel.set_fading_border(color(model.fading_border));
        self.wheel.set_has_thumbnail(model.has_thumbnail);
        if let Some(thumb) = &model.thumbnail {
            self.wheel.set_file_thumbnail(thumb.clone());
        }
    }

    /// Draws the wheel into an RGBA image file for preview or verification.
    pub fn save_capture(&mut self, path: &std::path::Path) -> Result<(), String> {
        let _ = self.render();
        let (w, h) = (self.width, self.height);
        let mut rgba = image::RgbaImage::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let pixel = self.buffer[(y * w + x) as usize];
                let (a, r, g, b) = pixel.parts();
                rgba.put_pixel(x, y, image::Rgba([r, g, b, a]));
            }
        }
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        rgba.save(path).map_err(|e| e.to_string())
    }

    /// Draws the wheel if anything changed. Returns whether it did.
    pub fn render(&mut self) -> bool {
        let stride = self.width as usize;
        let mut buffer = std::mem::take(&mut self.buffer);
        let redrew = self.window.draw_if_needed(|renderer| {
            renderer.render(&mut buffer[..], stride);
        });
        self.buffer = buffer;
        redrew
    }

    /// The pixels of the last frame.
    pub fn pixels(&self) -> &[u32] {
        let count = self.buffer.len();
        // SAFETY: `Bgra8` is a transparent wrapper around one `u32`.
        unsafe { std::slice::from_raw_parts(self.buffer.as_ptr() as *const u32, count) }
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// Asks for a complete redraw, for instance after the wheel has been
    /// hidden and shown again.
    pub fn request_redraw(&self) {
        self.window.request_redraw();
    }
}

/// Loads a downscaled thumbnail image for hub mode display.
#[allow(dead_code)]
pub fn load_thumbnail_image(path: &std::path::Path) -> Option<slint::Image> {
    if let Ok(img) = image::open(path) {
        let thumb = img.thumbnail(64, 64).to_rgba8();
        let (w, h) = thumb.dimensions();
        let buffer = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(
            thumb.as_raw(),
            w,
            h,
        );
        Some(slint::Image::from_rgba8(buffer))
    } else {
        None
    }
}

// The geometry module keeps its own `Point`; the view model needs it here.
use crate::radial::geometry::Point;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::radial::actions::{IconKind, RadialMode, Segment};

    /// A choice for the wheel.
    fn segment(index: usize) -> Segment {
        Segment {
            id: format!("test.{index}"),
            label: format!("Test {index}"),
            detail: "What it does".to_string(),
            icon: IconKind::Picture,
        }
    }

    /// The options the view model is built with.
    fn options(appear: f32) -> ViewOptions {
        ViewOptions {
            scale: 1.0,
            appear,
            emphasis: 0.0,
            fading: 0.0,
            emphasis_index: None,
            fading_index: None,
            notice: None,
            notice_appear: 0.0,
            theme: Theme::dark(),
            preview_pressed: false,
            has_thumbnail: false,
            thumbnail: None,
        }
    }

    /// The model of a wheel of `count` choices, with one of them pointed at.
    fn model_with(count: usize, highlighted: Option<usize>, theme: Theme) -> ViewModel {
        let segments: Vec<Segment> = (0..count).map(segment).collect();
        let mut options = options(1.0);
        options.theme = theme;
        options.emphasis_index = highlighted;
        build_view_model(
            &segments,
            RadialMode::Convert,
            highlighted,
            count,
            false,
            RadialGeometry::new(count),
            options,
        )
    }

    #[test]
    fn the_model_holds_the_choices_it_was_given() {
        // The number of choices on the wheel follows the files that were
        // dragged, so the model has to hold any number of them.
        for count in [0, 1, 2, 3, 4, 5, 8] {
            let segments: Vec<Segment> = (0..count).map(segment).collect();
            let model = build_view_model(
                &segments,
                RadialMode::Convert,
                None,
                count,
                false,
                RadialGeometry::new(count),
                options(1.0),
            );
            assert_eq!(
                model.segments.len(),
                count,
                "{count} choices were asked for"
            );
            for (index, data) in model.segments.iter().enumerate() {
                assert_eq!(data.label, format!("Test {index}"));
            }
        }
    }

    #[test]
    fn a_choice_that_is_not_pointed_at_is_a_neutral_one() {
        // Every theme has its own neutral colour for a choice, and the name
        // and the picture on it are the primary text of the theme.
        for theme in [Theme::light(), Theme::dark()] {
            let model = model_with(6, None, theme);
            for data in &model.segments {
                assert_eq!(data.fill, color(theme.segment));
                assert_eq!(data.label_color, color(theme.text));
                assert_eq!(data.icon_color, color(theme.text));
            }
        }
    }

    #[test]
    fn the_choice_the_pointer_is_on_is_filled_with_the_accent() {
        // The accent is in one piece: the colour of the theme, with no
        // transparency over it, so that there is no glow.
        for theme in [Theme::light(), Theme::dark()] {
            let model = model_with(6, Some(2), theme);
            assert_eq!(model.emphasis_color, theme.accent_hover);
            // The accent is opaque, so what is under it does not show.
            assert_eq!(model.emphasis_color.3, 0xff);
            assert_eq!(model.emphasis_border, theme.accent_hover);
        }
    }

    #[test]
    fn the_choice_the_pointer_is_on_has_white_words_on_it() {
        for theme in [Theme::light(), Theme::dark()] {
            let model = model_with(6, Some(2), theme);
            let pointed = &model.segments[2];
            assert_eq!(pointed.label_color, color(theme.accent_text()));
            assert_eq!(pointed.icon_color, color(theme.accent_text()));
            // And the other choices keep the colour of the theme.
            for (index, data) in model.segments.iter().enumerate() {
                if index == 2 {
                    continue;
                }
                assert_eq!(data.label_color, color(theme.text));
                assert_eq!(data.icon_color, color(theme.text));
            }
        }
    }

    #[test]
    fn a_choice_that_is_pressed_uses_the_pressed_accent() {
        for theme in [Theme::light(), Theme::dark()] {
            let mut options = options(1.0);
            options.theme = theme;
            options.preview_pressed = true;
            options.emphasis_index = Some(1);
            let segments: Vec<Segment> = (0..6).map(segment).collect();
            let model = build_view_model(
                &segments,
                RadialMode::Convert,
                Some(1),
                6,
                false,
                RadialGeometry::new(6),
                options,
            );
            assert_eq!(model.emphasis_color, theme.accent_pressed);
            // The words on it are still white, so that they can be read.
            assert_eq!(model.segments[1].label_color, color(theme.accent_text()));
        }
    }

    #[test]
    fn moving_the_pointer_puts_the_previous_choice_back() {
        // The choice that was pointed at is a choice again when the pointer
        // is on another one.
        let theme = Theme::dark();
        let model = model_with(6, Some(3), theme);
        assert_eq!(model.segments[3].label_color, color(theme.accent_text()));
        for (index, data) in model.segments.iter().enumerate() {
            if index == 3 {
                continue;
            }
            assert_eq!(data.fill, color(theme.segment));
            assert_eq!(data.label_color, color(theme.text));
        }
    }

    #[test]
    fn the_choice_that_is_losing_the_pointer_goes_back_to_being_a_choice() {
        let theme = Theme::dark();
        let model = model_with(6, Some(3), theme);
        // It is a little darker while it does, and it keeps its own edge.
        assert_eq!(model.fading_color, theme.hover);
        assert_eq!(model.fading_border, theme.border);
    }

    #[test]
    fn both_themes_draw_the_choices_the_same_way() {
        // The same rules in both themes: a neutral choice, an accent on the
        // one the pointer is on, and white words on it.
        let light = model_with(6, Some(0), Theme::light());
        let dark = model_with(6, Some(0), Theme::dark());
        assert_eq!(light.emphasis_color, Theme::light().accent_hover);
        assert_eq!(dark.emphasis_color, Theme::dark().accent_hover);
        assert_eq!(
            light.segments[0].label_color,
            color(Theme::light().accent_text())
        );
        assert_eq!(
            dark.segments[0].label_color,
            color(Theme::dark().accent_text())
        );
        // The geometry is the same in both.
        assert_eq!(light.segments.len(), dark.segments.len());
        for (a, b) in light.segments.iter().zip(dark.segments.iter()) {
            assert_eq!(a.path, b.path);
        }
    }

    #[test]
    fn the_wheel_is_drawn_with_the_colours_of_the_theme() {
        // The wheel is drawn in the theme it is given, so the colours of the
        // theme are the colours that are drawn.
        for theme in [Theme::light(), Theme::dark()] {
            let mut options = options(1.0);
            options.theme = theme;
            let model = build_view_model(
                &[segment(0)],
                RadialMode::Convert,
                Some(0),
                1,
                false,
                RadialGeometry::new(1),
                options,
            );
            assert_eq!(model.disc_color, theme.background);
            assert_eq!(model.hub_color, theme.surface);
            assert_eq!(model.border_color, theme.border);
            assert_eq!(
                model.segments[0].fill,
                slint::Color::from_argb_u8(
                    theme.segment.3,
                    theme.segment.0,
                    theme.segment.1,
                    theme.segment.2
                )
            );
            // The choice that is pointed at has the accent's words on it.
            assert_eq!(
                model.segments[0].label_color,
                slint::Color::from_argb_u8(
                    theme.accent_text().3,
                    theme.accent_text().0,
                    theme.accent_text().1,
                    theme.accent_text().2
                )
            );
        }
    }

    #[test]
    fn a_wheel_with_nothing_on_it_still_has_a_hub() {
        let model = build_view_model(
            &[],
            RadialMode::Convert,
            None,
            0,
            false,
            RadialGeometry::new(0),
            options(1.0),
        );
        assert!(model.segments.is_empty());
        assert_eq!(model.heading, "CONVERT");
        assert_eq!(model.subtitle, "No files");
    }

    #[test]
    fn the_hub_says_how_many_files_there_are() {
        let segments = vec![segment(0)];
        for (count, expected) in [(0, "No files"), (1, "1 file"), (3, "3 files")] {
            let model = build_view_model(
                &segments,
                RadialMode::Convert,
                None,
                count,
                false,
                RadialGeometry::new(1),
                options(1.0),
            );
            assert_eq!(model.subtitle, expected);
        }
    }

    #[test]
    fn thumbnail_hub_mode_is_represented_in_view_model() {
        let mut opts = options(1.0);
        opts.has_thumbnail = true;
        let segments = vec![segment(0)];
        let model = build_view_model(
            &segments,
            RadialMode::Convert,
            None,
            1,
            false,
            RadialGeometry::new(1),
            opts,
        );
        assert!(model.has_thumbnail);
    }

    #[test]
    fn capture_the_wheel_light_and_dark() {
        let mut ui_light = WheelUi::new(1.0).expect("wheel ui can be created");
        let model_light = model_with(6, Some(1), Theme::light());
        ui_light.apply(&model_light);
        let shot_light = std::path::Path::new("target").join("shots").join("wheel-light.png");
        ui_light.save_capture(&shot_light).expect("wheel light captured");
        assert!(shot_light.is_file(), "wheel-light.png written");

        let mut ui_dark = WheelUi::new(1.0).expect("wheel ui can be created");
        let model_dark = model_with(6, Some(1), Theme::dark());
        ui_dark.apply(&model_dark);
        let shot_dark = std::path::Path::new("target").join("shots").join("wheel-dark.png");
        ui_dark.save_capture(&shot_dark).expect("wheel dark captured");
        assert!(shot_dark.is_file(), "wheel-dark.png written");
    }
}
