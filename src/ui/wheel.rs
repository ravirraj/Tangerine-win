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
use crate::radial::geometry::RadialGeometry;
use crate::radial::icons;

slint::include_modules!();

/// The width of the wheel in logical pixels, before the display's own scale.
pub const DISC_SIZE: f32 = 380.0;

/// The radius of the empty hub, in logical pixels.
pub const HUB_SIZE: f32 = 128.0;

/// The near black the wheel is drawn on.
const SURFACE: (u8, u8, u8, u8) = (0x14, 0x14, 0x19, 0xff);
/// The hairline between two segments.
const HAIRLINE: (u8, u8, u8, u8) = (0x2c, 0x2c, 0x35, 0xff);
/// The colour of a name that is not pointed at.
const LABEL: (u8, u8, u8, u8) = (0xc4, 0xc4, 0xcf, 0xff);
/// The colour of a name that is pointed at.
const LABEL_ACTIVE: (u8, u8, u8, u8) = (0xff, 0xff, 0xff, 0xff);
/// The colour of a picture that is not pointed at.
const ICON: (u8, u8, u8, u8) = (0x76, 0x76, 0x84, 0xff);
/// The colour of a picture that is pointed at.
const ICON_ACTIVE: (u8, u8, u8, u8) = (0xff, 0xb2, 0x7a, 0xff);

/// A pixel in the order `UpdateLayeredWindow` wants: blue, green, red, alpha.
#[repr(transparent)]
#[derive(Clone, Copy, Default)]
pub struct Bgra8(u32);

impl Bgra8 {
    fn parts(self) -> (u8, u8, u8, u8) {
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
struct WheelPlatform {
    window: Rc<MinimalSoftwareWindow>,
}

impl Platform for WheelPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.window.clone())
    }
}

thread_local! {
    static WINDOW: Rc<MinimalSoftwareWindow> =
        MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
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

    let mut geometry = geometry;
    geometry.inner_radius *= appear;
    geometry.outer_radius *= appear;

    let centre = Point::new(disc_size / 2.0, disc_size / 2.0);
    let mut data = Vec::with_capacity(segments.len());

    for (index, segment) in segments.iter().enumerate() {
        let active = highlighted == Some(index);
        let label_point = geometry.label_point(index);
        let icon_size = 22.0 * scale * appear;
        // The picture sits above the name, in a straight line, so that the
        // pair reads the same way on every segment.
        let icon_centre = Point::new(label_point.x, label_point.y - 15.0 * scale * appear);
        let text_centre = Point::new(label_point.x, label_point.y + 10.0 * scale * appear);

        data.push(SegmentData {
            path: geometry.wedge_path(index).into(),
            fill: color(SURFACE),
            stroke: color(HAIRLINE),
            label: segment.label.clone().into(),
            label_color: color(if active { LABEL_ACTIVE } else { LABEL }),
            label_size: (if active { 12.0 } else { 11.0 }) * scale * appear,
            label_x: centre.x + text_centre.x,
            label_y: centre.y + text_centre.y,
            label_width: geometry.label_width() * scale,
            icon: icons::path_for(segment.icon).into(),
            icon_color: color(if active { ICON_ACTIVE } else { ICON }),
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
        hub_size: HUB_SIZE * scale * appear,
        heading_size: 12.0 * scale * appear,
        subtitle_size: 10.0 * scale * appear,
        notice_text: options
            .notice
            .as_ref()
            .map(|notice| notice.text.clone())
            .unwrap_or_default(),
        notice_failed: options.notice.as_ref().is_some_and(|notice| notice.failed),
        notice_appear: options.notice_appear,
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
        let window = WINDOW.with(|window| window.clone());
        // Installing the platform only works once per process; a second call
        // simply keeps the first one, which is what we want.
        let _ = slint::platform::set_platform(Box::new(WheelPlatform {
            window: window.clone(),
        }));

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
        }
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
}
