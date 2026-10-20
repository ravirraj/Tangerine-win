//! Geometry for the radial wheel.
//!
//! Everything here is pure maths with no dependency on the UI framework, so the
//! hit testing and the wedge outlines can be unit tested on their own.
//!
//! Angle convention: degrees, measured **clockwise from up** (12 o'clock is 0°,
//! 3 o'clock is 90°, 6 o'clock is 180°). Screen coordinates have `y` growing
//! downwards, so a point at radius `r` and angle `a` is
//! `(r * sin(a), -r * cos(a))` relative to the wheel centre.

/// A point in wheel space (pixels, `y` downwards).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    /// Distance from the origin.
    pub fn length(self) -> f32 {
        (self.x * self.x + self.y * self.y).sqrt()
    }
}

/// The shape of the wheel: an annulus cut into `count` segments.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RadialGeometry {
    /// Radius of the empty hub in the middle.
    pub inner_radius: f32,
    /// Radius of the outer edge of the segments.
    pub outer_radius: f32,
    /// Angular gap left between two neighbouring segments, in degrees.
    pub gap_degrees: f32,
    /// How many segments the wheel is cut into.
    pub count: usize,
}

impl RadialGeometry {
    /// The geometry the product uses for a wheel of `count` segments.
    pub fn new(count: usize) -> Self {
        Self {
            inner_radius: 64.0,
            outer_radius: 150.0,
            gap_degrees: 4.0,
            count,
        }
    }

    /// The angular width of one segment, in degrees.
    pub fn slice_degrees(&self) -> f32 {
        if self.count == 0 {
            0.0
        } else {
            360.0 / self.count as f32
        }
    }

    /// The angular span of segment `index`, gaps included.
    ///
    /// Returns `(0.0, 0.0)` for an out-of-range index.
    pub fn segment_span(&self, index: usize) -> (f32, f32) {
        if index >= self.count {
            return (0.0, 0.0);
        }
        let slice = self.slice_degrees();
        let half_gap = self.gap_degrees.min(slice) / 2.0;
        let start = index as f32 * slice + half_gap;
        let end = (index as f32 + 1.0) * slice - half_gap;
        (start, end)
    }

    /// The angle at the middle of segment `index`.
    pub fn mid_angle(&self, index: usize) -> f32 {
        let (start, end) = self.segment_span(index);
        (start + end) / 2.0
    }

    /// The radius the segment labels sit at.
    pub fn label_radius(&self) -> f32 {
        self.inner_radius + (self.outer_radius - self.inner_radius) * 0.56
    }

    /// The widest a label may be before it runs into its neighbours.
    pub fn label_width(&self) -> f32 {
        let chord = 2.0 * self.label_radius() * (self.slice_degrees().to_radians() / 2.0).sin();
        (chord - 6.0).max(24.0)
    }

    /// The point at `radius` and `angle` (degrees clockwise from up), relative to the centre.
    pub fn point_at(&self, radius: f32, angle_degrees: f32) -> Point {
        let radians = angle_degrees.to_radians();
        Point::new(radius * radians.sin(), -radius * radians.cos())
    }

    /// Where the label of segment `index` goes, relative to the centre.
    pub fn label_point(&self, index: usize) -> Point {
        self.point_at(self.label_radius(), self.mid_angle(index))
    }

    /// The angle of a point relative to the centre, in degrees clockwise from up.
    pub fn angle_of(&self, offset: Point) -> f32 {
        let mut angle = offset.x.atan2(-offset.y).to_degrees();
        if angle < 0.0 {
            angle += 360.0;
        }
        if angle >= 360.0 {
            angle -= 360.0;
        }
        angle
    }

    /// The same wheel, but drawn at the scale of the display.
    pub fn scaled(&self, scale: f32) -> RadialGeometry {
        let mut copy = *self;
        copy.inner_radius *= scale;
        copy.outer_radius *= scale;
        copy
    }

    /// Works out where the wheel goes for a pointer, and how big it is.
    ///
    /// `radius` is half the size of the whole wheel, in physical pixels. The
    /// wheel is kept on the screen, so a pointer near an edge puts the wheel
    /// against the edge rather than half off it.
    pub fn placement(
        &self,
        pointer: Point,
        radius: f32,
        bounds: (i32, i32, i32, i32),
    ) -> WheelPlacement {
        WheelPlacement {
            centre: clamp_center(pointer, radius, bounds),
            geometry: *self,
        }
    }

    /// The hit test, with a little patience.
    ///
    /// The gap between two segments belongs to the one on its clockwise side,
    /// so a pointer resting on the line between two of them would make the
    /// choice jump from one to the other. The choice the pointer already had is
    /// therefore kept while the pointer is still inside the gap.
    ///
    /// The same goes for the edge of the hub and the edge of the rim: a
    /// pointer resting on one of those lines would make the choice come and go.
    /// The dead zone is as wide as the gap is at the inner edge, so it is the
    /// same size everywhere.
    ///
    /// The patience never keeps a segment the pointer has clearly left: once
    /// the pointer is further than the dead zone from the line, the plain hit
    /// test wins.
    pub fn hit_test_steady(&self, offset: Point, current: Option<usize>) -> Option<usize> {
        let raw = self.hit_test(offset);
        match (current, raw) {
            (None, raw) => raw,
            (Some(current), Some(raw)) => {
                if current == raw {
                    return Some(raw);
                }
                // The pointer is on the line between the two segments. The
                // line is the one they share.
                let slice = self.slice_degrees();
                let boundary = if raw > current {
                    (current + 1) as f32 * slice
                } else {
                    current as f32 * slice
                };
                let mut away = (self.angle_of(offset) - boundary).abs();
                if away > 180.0 {
                    away = 360.0 - away;
                }
                if away <= self.gap_degrees / 2.0 {
                    Some(current)
                } else {
                    Some(raw)
                }
            }
            (Some(current), None) => {
                // The pointer is on the edge of the hub or of the rim.
                let distance = offset.length();
                if distance >= self.inner_radius - self.dead_zone()
                    && distance <= self.outer_radius + self.dead_zone()
                {
                    Some(current)
                } else {
                    None
                }
            }
        }
    }

    /// How wide the dead zone at the edge of the hub and of the rim is.
    ///
    /// It is the gap, measured at the inner edge, so that it is the same size
    /// as the gap between two segments.
    pub fn dead_zone(&self) -> f32 {
        self.gap_degrees.to_radians() * self.inner_radius
    }

    /// Which segment a point falls in.
    ///
    /// `None` means the point is on the hub, outside the wheel, or the wheel has
    /// no segments. The angular gap between two segments belongs to the segment
    /// on its clockwise side, which keeps the pointer from flickering between two
    /// neighbours while it hovers over the seam.
    pub fn hit_test(&self, offset: Point) -> Option<usize> {
        if self.count == 0 {
            return None;
        }
        let distance = offset.length();
        if !distance.is_finite() {
            return None;
        }
        if distance < self.inner_radius || distance > self.outer_radius {
            return None;
        }
        let slice = self.slice_degrees();
        if slice <= 0.0 {
            return None;
        }
        let index = (self.angle_of(offset) / slice).floor();
        if !index.is_finite() {
            return None;
        }
        let index = index as usize;
        Some(index.min(self.count - 1))
    }

    /// The corner radius used for the rounded ends of a segment.
    ///
    /// Wider wheels with fewer segments get slightly rounder corners, but never so
    /// round that the straight edges disappear.
    pub fn corner_radius(&self) -> f32 {
        if self.count == 0 {
            return 0.0;
        }
        let wanted: f32 = if self.count > 8 { 10.0 } else { 14.0 };
        let limit = self.inner_radius * (self.slice_degrees().to_radians() * 0.34).sin();
        wanted.min(limit).max(4.0)
    }

    /// The outline of segment `index` as an SVG path, centred on the origin.
    ///
    /// The path is a rounded annular sector: two circular arcs joined by two
    /// straight radial edges, with the four corners filleted.
    pub fn wedge_path(&self, index: usize) -> String {
        if index >= self.count {
            return String::new();
        }
        let (start, end) = self.segment_span(index);
        let r0 = self.inner_radius;
        let r1 = self.outer_radius;
        let k = self.corner_radius().min(r1 / 2.5).min((r1 - r0) / 3.0);

        // The fillet at the outer edge: a circle of radius `k` tangent to the
        // outer arc and to the radial edge.
        let delta_outer = (k / (r1 - k)).clamp(-1.0, 1.0).asin();
        // The fillet at the inner edge: a circle of radius `k` tangent to the
        // inner arc and to the radial edge, this time on the other side.
        let delta_inner = (k / (r0 + k)).clamp(-1.0, 1.0).asin();

        let outer_start = start + delta_outer.to_degrees();
        let outer_end = end - delta_outer.to_degrees();
        let inner_start = start + delta_inner.to_degrees();
        let inner_end = end - delta_inner.to_degrees();

        // Where the radial edges touch the fillets.
        let outer_tangent_start = (r1 - k) * delta_outer.cos();
        let outer_tangent_end = (r1 - k) * delta_outer.cos();
        let inner_tangent_start = (r0 + k) * delta_inner.cos();
        let inner_tangent_end = (r0 + k) * delta_inner.cos();

        let mut path = String::with_capacity(256);

        let p_outer_start = self.point_at(r1, outer_start);
        push_move(&mut path, p_outer_start);

        // Outer arc, clockwise.
        let p_outer_end = self.point_at(r1, outer_end);
        push_arc(&mut path, r1, p_outer_end);

        // Corner at the clockwise end of the outer edge.
        let p_outer_tangent_end = self.point_at(outer_tangent_end, end);
        push_arc(&mut path, k, p_outer_tangent_end);

        // Straight edge down to the inner radius.
        let p_inner_tangent_end = self.point_at(inner_tangent_end, end);
        push_line(&mut path, p_inner_tangent_end);

        // Corner at the clockwise end of the inner edge.
        let p_inner_end = self.point_at(r0, inner_end);
        push_arc(&mut path, k, p_inner_end);

        // Inner arc, back the other way.
        let p_inner_start = self.point_at(r0, inner_start);
        push_arc_reverse(&mut path, r0, p_inner_start);

        // Corner at the counter-clockwise end of the inner edge.
        let p_inner_tangent_start = self.point_at(inner_tangent_start, start);
        push_arc(&mut path, k, p_inner_tangent_start);

        // Straight edge back out to the outer radius.
        let p_outer_tangent_start = self.point_at(outer_tangent_start, start);
        push_line(&mut path, p_outer_tangent_start);

        // Corner at the counter-clockwise end of the outer edge.
        push_arc(&mut path, k, p_outer_start);

        path.push('Z');
        path
    }
}

fn number(value: f32) -> String {
    // Two decimals is plenty for a path and keeps the strings short.
    let rounded = (value * 100.0).round() / 100.0;
    if rounded == 0.0 {
        "0".to_string()
    } else {
        let text = format!("{rounded:.2}");
        text.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

fn push_move(path: &mut String, point: Point) {
    path.push_str("M ");
    path.push_str(&number(point.x));
    path.push(' ');
    path.push_str(&number(point.y));
}

fn push_line(path: &mut String, point: Point) {
    path.push_str(" L ");
    path.push_str(&number(point.x));
    path.push(' ');
    path.push_str(&number(point.y));
}

/// A circular arc from the current point to `to`, drawn clockwise.
///
/// Used both for the wheel's own outer and inner circles and for the short
/// corner fillets, which always sweep less than a quarter turn.
fn push_arc(path: &mut String, radius: f32, to: Point) {
    path.push_str(" A ");
    path.push_str(&number(radius));
    path.push(' ');
    path.push_str(&number(radius));
    path.push_str(" 0 0 1 ");
    path.push_str(&number(to.x));
    path.push(' ');
    path.push_str(&number(to.y));
}

fn push_arc_reverse(path: &mut String, radius: f32, to: Point) {
    path.push_str(" A ");
    path.push_str(&number(radius));
    path.push(' ');
    path.push_str(&number(radius));
    path.push_str(" 0 0 0 ");
    path.push_str(&number(to.x));
    path.push(' ');
    path.push_str(&number(to.y));
}

/// Where the wheel is on the screen and how big it is.
///
/// Both are in **physical** screen pixels, which is what the window placement
/// and the pointer both use. The drawing and the hit testing have to use these
/// very numbers: if the hit test uses a different centre or different radii
/// from the drawing, the wheel lights up in the wrong place.
#[derive(Clone, Copy, Debug)]
pub struct WheelPlacement {
    /// The centre of the wheel, in physical screen pixels.
    pub centre: Point,
    /// The shape of the wheel, in physical screen pixels.
    pub geometry: RadialGeometry,
}

impl WheelPlacement {
    /// The segment a screen point is over, with the patience of
    /// [`RadialGeometry::hit_test_steady`].
    pub fn segment_at(&self, screen: Point, current: Option<usize>) -> Option<usize> {
        let offset = Point::new(screen.x - self.centre.x, screen.y - self.centre.y);
        self.geometry.hit_test_steady(offset, current)
    }
}

/// Keeps the wheel fully on screen when it is centred on the cursor.
///
/// Returns the corrected centre for a wheel of `radius` on a screen of
/// `bounds`, where `bounds` is `(left, top, right, bottom)` in screen pixels.
pub fn clamp_center(center: Point, radius: f32, bounds: (i32, i32, i32, i32)) -> Point {
    let (left, top, right, bottom) = bounds;
    let min_x = left as f32 + radius;
    let max_x = right as f32 - radius;
    let min_y = top as f32 + radius;
    let max_y = bottom as f32 - radius;

    let mut x = center.x;
    let mut y = center.y;
    x = if min_x > max_x {
        (min_x + max_x) / 2.0
    } else {
        x.clamp(min_x, max_x)
    };
    y = if min_y > max_y {
        (min_y + max_y) / 2.0
    } else {
        y.clamp(min_y, max_y)
    };
    Point::new(x, y)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn geometry(count: usize) -> RadialGeometry {
        RadialGeometry::new(count)
    }

    fn at(degrees: f32, radius: f32) -> Point {
        let radians = degrees.to_radians();
        Point::new(radius * radians.sin(), -radius * radians.cos())
    }

    #[test]
    fn eight_segments_have_a_45_degree_slice() {
        let g = geometry(8);
        assert_eq!(g.slice_degrees(), 45.0);
    }

    #[test]
    fn cardinal_directions_pick_the_right_segment() {
        let g = geometry(8);
        assert_eq!(g.hit_test(at(0.0, 100.0)), Some(0));
        assert_eq!(g.hit_test(at(90.0, 100.0)), Some(2));
        assert_eq!(g.hit_test(at(180.0, 100.0)), Some(4));
        assert_eq!(g.hit_test(at(270.0, 100.0)), Some(6));
    }

    #[test]
    fn diagonal_directions_pick_the_right_segment() {
        let g = geometry(8);
        assert_eq!(g.hit_test(at(44.0, 100.0)), Some(0));
        assert_eq!(g.hit_test(at(46.0, 100.0)), Some(1));
        assert_eq!(g.hit_test(at(134.0, 100.0)), Some(2));
        assert_eq!(g.hit_test(at(136.0, 100.0)), Some(3));
        assert_eq!(g.hit_test(at(224.0, 100.0)), Some(4));
        assert_eq!(g.hit_test(at(226.0, 100.0)), Some(5));
        assert_eq!(g.hit_test(at(314.0, 100.0)), Some(6));
        assert_eq!(g.hit_test(at(316.0, 100.0)), Some(7));
    }

    #[test]
    fn a_segment_boundary_belongs_to_the_clockwise_side() {
        let g = geometry(8);
        assert_eq!(g.hit_test(at(45.0, 100.0)), Some(1));
        assert_eq!(g.hit_test(at(90.0, 100.0)), Some(2));
    }

    #[test]
    fn the_hub_is_never_a_segment() {
        let g = geometry(8);
        assert_eq!(g.hit_test(Point::new(0.0, 0.0)), None);
        assert_eq!(g.hit_test(at(0.0, g.inner_radius - 0.5)), None);
        assert_eq!(g.hit_test(at(37.0, g.inner_radius - 0.5)), None);
    }

    #[test]
    fn the_outer_edge_is_never_a_segment() {
        let g = geometry(8);
        assert_eq!(g.hit_test(at(0.0, g.outer_radius + 0.5)), None);
        assert_eq!(g.hit_test(at(0.0, g.outer_radius)), Some(0));
    }

    #[test]
    fn a_wheel_without_segments_never_hits() {
        let g = geometry(0);
        assert_eq!(g.hit_test(at(0.0, 100.0)), None);
        assert_eq!(g.segment_span(0), (0.0, 0.0));
        assert!(g.wedge_path(0).is_empty());
    }

    #[test]
    fn hit_testing_is_stable_at_the_centre_of_the_screen() {
        let g = geometry(8);
        assert_eq!(g.hit_test(Point::new(0.0, 0.0)), None);
        assert_eq!(g.hit_test(Point::new(-0.0, -0.0)), None);
    }

    #[test]
    fn non_finite_coordinates_do_not_panic() {
        let g = geometry(8);
        assert_eq!(g.hit_test(Point::new(f32::NAN, 0.0)), None);
        assert_eq!(g.hit_test(Point::new(f32::INFINITY, 0.0)), None);
    }

    #[test]
    fn the_gap_splits_the_slice() {
        let g = geometry(8);
        let (start, end) = g.segment_span(0);
        assert!((start - 2.0).abs() < 0.001);
        assert!((end - 43.0).abs() < 0.001);
    }

    #[test]
    fn labels_sit_between_the_hub_and_the_rim() {
        let g = geometry(8);
        let radius = g.label_radius();
        assert!(radius > g.inner_radius && radius < g.outer_radius);
        // Every label sits in the middle of its own segment.
        for index in 0..8 {
            let expected = g.point_at(radius, g.mid_angle(index));
            let actual = g.label_point(index);
            assert!((actual.x - expected.x).abs() < 0.01);
            assert!((actual.y - expected.y).abs() < 0.01);
        }
        // The first segment is at the top of the wheel and the fifth at the
        // bottom, half a segment to either side of straight up.
        assert!(g.label_point(0).y < 0.0);
        assert!(g.label_point(4).y > 0.0);
    }

    #[test]
    fn the_wedge_path_is_a_closed_shape() {
        let g = geometry(8);
        let path = g.wedge_path(0);
        assert!(path.starts_with("M "));
        assert!(path.ends_with('Z'));
        assert!(path.contains(" A "));
        assert!(path.contains(" L "));
    }

    #[test]
    fn every_wedge_path_is_different() {
        let g = geometry(8);
        let first = g.wedge_path(0);
        for index in 1..8 {
            assert_ne!(first, g.wedge_path(index));
        }
    }

    #[test]
    fn the_wedge_path_stays_inside_the_wheel() {
        let g = geometry(8);
        for index in 0..8 {
            let path = g.wedge_path(index);
            for point in path_points(&path) {
                let distance = (point.x * point.x + point.y * point.y).sqrt();
                assert!(
                    distance <= g.outer_radius + 0.5,
                    "segment {index} reaches {distance}, past the rim at {}",
                    g.outer_radius
                );
                assert!(
                    distance >= g.inner_radius - 0.5,
                    "segment {index} reaches into the hub at {distance}"
                );
            }
        }
    }

    /// The points an SVG path visits.
    fn path_points(path: &str) -> Vec<Point> {
        let mut points = Vec::new();
        let mut numbers: Vec<f32> = Vec::new();
        let mut expected = 0usize;
        for word in path.split_whitespace() {
            match word {
                "M" | "L" => expected = 2,
                "A" => expected = 7,
                "Z" | "z" => expected = 0,
                _ => {
                    if let Ok(value) = word.parse::<f32>() {
                        numbers.push(value);
                        if numbers.len() == expected {
                            let count = numbers.len();
                            points.push(Point::new(numbers[count - 2], numbers[count - 1]));
                            numbers.clear();
                        }
                    }
                }
            }
        }
        points
    }

    #[test]
    fn a_scaled_wheel_keeps_its_shape() {
        let g = geometry(8);
        let scaled = g.scaled(1.5);
        assert_eq!(scaled.count, 8);
        assert_eq!(scaled.inner_radius, g.inner_radius * 1.5);
        assert_eq!(scaled.outer_radius, g.outer_radius * 1.5);
        assert_eq!(scaled.slice_degrees(), g.slice_degrees());
    }

    #[test]
    fn the_placement_keeps_the_wheel_on_the_screen() {
        let g = geometry(8);
        let bounds = (0, 0, 1920, 1080);
        // A pointer in the corner: the wheel is pushed back onto the screen.
        let placement = g.placement(Point::new(10.0, 10.0), 190.0, bounds);
        assert_eq!(placement.centre, Point::new(190.0, 190.0));
        // A pointer in the middle: the wheel is left where it is.
        let placement = g.placement(Point::new(960.0, 540.0), 190.0, bounds);
        assert_eq!(placement.centre, Point::new(960.0, 540.0));
    }

    #[test]
    fn the_placement_keeps_the_geometry_it_was_given() {
        let g = geometry(8).scaled(1.5);
        let placement = g.placement(Point::new(960.0, 540.0), 285.0, (0, 0, 1920, 1080));
        assert_eq!(placement.geometry.outer_radius, g.outer_radius);
        assert_eq!(placement.geometry.inner_radius, g.inner_radius);
    }

    #[test]
    fn the_patience_keeps_the_choice_at_a_line() {
        let g = geometry(8);
        // The line between the first and the second segment is at 45 degrees.
        let just_past = at(45.5, 100.0);
        let raw = g.hit_test(just_past);
        assert_eq!(raw, Some(1));
        // The pointer was on the first segment, so it stays there while it is
        // still inside the gap.
        assert_eq!(g.hit_test_steady(just_past, Some(0)), Some(0));
        // Once the pointer has clearly moved on, the choice follows it.
        assert_eq!(g.hit_test_steady(at(60.0, 100.0), Some(0)), Some(1));
    }

    #[test]
    fn the_patience_does_not_hold_a_segment_the_pointer_has_left() {
        let g = geometry(8);
        // The pointer is well past the first segment, on the one before it.
        let index = g.hit_test_steady(at(350.0, 100.0), Some(0));
        assert_eq!(index, Some(7));
    }

    #[test]
    fn the_patience_says_nothing_when_there_is_no_choice() {
        let g = geometry(8);
        // On the hub.
        assert_eq!(g.hit_test_steady(Point::new(0.0, 0.0), Some(3)), None);
        // Outside the wheel.
        assert_eq!(g.hit_test_steady(at(0.0, 400.0), Some(3)), None);
        // There was no choice to begin with.
        assert_eq!(g.hit_test_steady(at(45.5, 100.0), None), Some(1));
    }

    #[test]
    fn the_patience_covers_the_gap_between_two_segments() {
        let g = geometry(8);
        // The gap is four degrees wide, so the patience is two degrees.
        assert_eq!(g.gap_degrees / 2.0, 2.0);
        // A pointer right on the line stays where it was.
        assert_eq!(g.hit_test_steady(at(45.0, 100.0), Some(0)), Some(0));
        assert_eq!(g.hit_test_steady(at(45.0, 100.0), Some(1)), Some(1));
    }

    #[test]
    fn the_patience_covers_the_edge_of_the_hub() {
        let g = geometry(8);
        // Just inside the hub: the choice stays.
        let just_inside = Point::new(0.0, -(g.inner_radius - 1.0));
        assert_eq!(g.hit_test(just_inside), None);
        assert_eq!(g.hit_test_steady(just_inside, Some(3)), Some(3));
        // Well inside the hub: the choice is given up.
        let well_inside = Point::new(0.0, -(g.inner_radius - 20.0));
        assert_eq!(g.hit_test_steady(well_inside, Some(3)), None);
    }

    #[test]
    fn the_patience_covers_the_edge_of_the_rim() {
        let g = geometry(8);
        // Just past the rim: the choice stays.
        let just_outside = Point::new(0.0, -(g.outer_radius + 1.0));
        assert_eq!(g.hit_test(just_outside), None);
        assert_eq!(g.hit_test_steady(just_outside, Some(3)), Some(3));
        // Well past the rim: the choice is given up.
        let well_outside = Point::new(0.0, -(g.outer_radius + 20.0));
        assert_eq!(g.hit_test_steady(well_outside, Some(3)), None);
    }

    #[test]
    fn the_dead_zone_is_the_gap_at_the_inner_edge() {
        let g = geometry(8);
        let expected = g.gap_degrees.to_radians() * g.inner_radius;
        assert!((g.dead_zone() - expected).abs() < 0.001);
    }

    #[test]
    fn the_placement_finds_the_segment_under_a_point() {
        let g = geometry(8);
        let placement = g.placement(Point::new(960.0, 540.0), 190.0, (0, 0, 1920, 1080));
        // Straight up from the centre of the wheel.
        let index = placement.segment_at(Point::new(960.0, 440.0), None);
        assert_eq!(index, Some(0));
        // On the hub.
        assert_eq!(placement.segment_at(Point::new(960.0, 540.0), None), None);
    }

    #[test]
    fn clamping_keeps_the_wheel_on_screen() {
        let bounds = (0, 0, 1920, 1080);
        let clamped = clamp_center(Point::new(10.0, 10.0), 200.0, bounds);
        assert_eq!(clamped, Point::new(200.0, 200.0));

        let clamped = clamp_center(Point::new(1910.0, 1070.0), 200.0, bounds);
        assert_eq!(clamped, Point::new(1720.0, 880.0));

        let clamped = clamp_center(Point::new(960.0, 540.0), 200.0, bounds);
        assert_eq!(clamped, Point::new(960.0, 540.0));
    }

    #[test]
    fn clamping_survives_a_screen_smaller_than_the_wheel() {
        let bounds = (0, 0, 100, 100);
        let clamped = clamp_center(Point::new(50.0, 50.0), 200.0, bounds);
        assert!(clamped.x.is_finite() && clamped.y.is_finite());
        assert_eq!(clamped, Point::new(50.0, 50.0));
    }
}
