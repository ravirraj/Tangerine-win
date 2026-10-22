//! The one animation model behind the wheel.
//!
//! Everything that moves is driven from here: the entrance, the dismissal and
//! the segment under the pointer. One model means one property is never fought
//! over by two animations.

/// How long the wheel takes to arrive, in seconds.
const APPEAR_IN: f32 = 0.18;
/// How long the wheel takes to leave, in seconds.
const APPEAR_OUT: f32 = 0.13;
/// How long a segment takes to light up, in seconds.
///
/// The selection transition takes 150-200 ms; this value sits in that
/// range so the wheel feels responsive without snapping.
const EMPHASIS_IN: f32 = 0.15;
/// How long a segment takes to fade out, in seconds.
///
/// Matches the light-up time so that entering and leaving a choice feel
/// balanced.
const EMPHASIS_OUT: f32 = 0.15;

/// Moves `current` towards `target`, never overshooting.
fn approach(current: f32, target: f32, speed: f32, delta: f32) -> (f32, bool) {
    let difference = target - current;
    let step = speed * delta;
    if difference.abs() <= step {
        (target, difference != 0.0)
    } else {
        (current + difference.signum() * step, true)
    }
}

/// The state of the wheel's motion.
#[derive(Clone, Debug)]
pub struct WheelAnimation {
    /// How far the entrance has got, from 0 to 1.
    appear: f32,
    /// Where the entrance is heading.
    appear_target: f32,
    /// How much the pointed-at segment has lit up.
    emphasis: f32,
    /// The segment that is lit up.
    emphasis_index: Option<usize>,
    /// How much the segment that is losing the pointer has faded.
    fading: f32,
    /// The segment that is fading.
    fading_index: Option<usize>,
}

impl Default for WheelAnimation {
    fn default() -> Self {
        Self::new()
    }
}

impl WheelAnimation {
    pub fn new() -> Self {
        Self {
            appear: 0.0,
            appear_target: 0.0,
            emphasis: 0.0,
            emphasis_index: None,
            fading: 0.0,
            fading_index: None,
        }
    }

    /// Starts the entrance.
    pub fn show(&mut self) {
        self.appear_target = 1.0;
    }

    /// Starts the dismissal.
    pub fn hide(&mut self) {
        self.appear_target = 0.0;
    }

    /// Whether the wheel has finished leaving.
    pub fn is_gone(&self) -> bool {
        self.appear_target == 0.0 && self.appear == 0.0
    }

    /// Points at a segment, or at nothing.
    pub fn set_highlight(&mut self, index: Option<usize>) {
        if index == self.emphasis_index {
            return;
        }
        if let Some(previous) = self.emphasis_index {
            self.fading_index = Some(previous);
            self.fading = self.emphasis;
        }
        self.emphasis_index = index;
        self.emphasis = 0.0;
        crate::diagnostics::log(
            crate::diagnostics::Level::Debug,
            "wheel",
            &format!("the drawing will now light up segment {index:?}"),
        );
    }

    pub fn appear(&self) -> f32 {
        self.appear
    }

    pub fn emphasis(&self) -> f32 {
        self.emphasis
    }

    pub fn emphasis_index(&self) -> Option<usize> {
        self.emphasis_index
    }

    pub fn fading(&self) -> f32 {
        self.fading
    }

    pub fn fading_index(&self) -> Option<usize> {
        self.fading_index
    }

    /// Advances the animation. Returns whether anything moved.
    pub fn update(&mut self, delta: f32) -> bool {
        let mut changed = false;

        let (appear, moved) = approach(
            self.appear,
            self.appear_target,
            if self.appear_target > self.appear {
                1.0 / APPEAR_IN
            } else {
                1.0 / APPEAR_OUT
            },
            delta,
        );
        self.appear = appear;
        changed |= moved;

        let emphasis_target = if self.emphasis_index.is_some() {
            1.0
        } else {
            0.0
        };
        let (emphasis, moved) = approach(
            self.emphasis,
            emphasis_target,
            if emphasis_target > self.emphasis {
                1.0 / EMPHASIS_IN
            } else {
                1.0 / EMPHASIS_OUT
            },
            delta,
        );
        self.emphasis = emphasis;
        changed |= moved;

        let (fading, moved) = approach(self.fading, 0.0, 1.0 / EMPHASIS_OUT, delta);
        self.fading = fading;
        changed |= moved;
        if self.fading == 0.0 {
            self.fading_index = None;
        }

        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_entrance_runs_from_nothing_to_everything() {
        let mut animation = WheelAnimation::new();
        assert_eq!(animation.appear(), 0.0);
        animation.show();
        for _ in 0..60 {
            animation.update(1.0 / 60.0);
        }
        assert_eq!(animation.appear(), 1.0);
    }

    #[test]
    fn the_dismissal_runs_back_to_nothing() {
        let mut animation = WheelAnimation::new();
        animation.show();
        for _ in 0..60 {
            animation.update(1.0 / 60.0);
        }
        animation.hide();
        for _ in 0..60 {
            animation.update(1.0 / 60.0);
        }
        assert_eq!(animation.appear(), 0.0);
        assert!(animation.is_gone());
    }

    #[test]
    fn pointing_at_a_segment_lights_it_up() {
        let mut animation = WheelAnimation::new();
        animation.set_highlight(Some(2));
        assert_eq!(animation.emphasis_index(), Some(2));
        for _ in 0..30 {
            animation.update(1.0 / 60.0);
        }
        assert_eq!(animation.emphasis(), 1.0);
    }

    #[test]
    fn moving_the_pointer_fades_the_old_segment_out() {
        let mut animation = WheelAnimation::new();
        animation.set_highlight(Some(2));
        for _ in 0..30 {
            animation.update(1.0 / 60.0);
        }
        animation.set_highlight(Some(5));
        assert_eq!(animation.emphasis_index(), Some(5));
        assert_eq!(animation.fading_index(), Some(2));
        assert!(animation.fading() > 0.0);
        for _ in 0..60 {
            animation.update(1.0 / 60.0);
        }
        assert_eq!(animation.fading(), 0.0);
        assert_eq!(animation.fading_index(), None);
    }

    #[test]
    fn the_animation_never_overshoots() {
        let mut animation = WheelAnimation::new();
        animation.show();
        animation.update(10.0);
        assert_eq!(animation.appear(), 1.0);
        animation.hide();
        animation.update(10.0);
        assert_eq!(animation.appear(), 0.0);
    }
}
