use serde::{Deserialize, Serialize};

/// Physical screen pixels; never store Explorer-relative coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Geometry {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl Geometry {
    /// Snap a fitted widget to the nearest work-area edge without changing its size.
    pub fn snap(self, work: Self, distance: i32) -> Self {
        fn axis(value: i32, near: i32, far: i32, distance: u32) -> i32 {
            let near_distance = value.abs_diff(near);
            let far_distance = value.abs_diff(far);
            if near_distance <= distance && near_distance <= far_distance {
                near
            } else if far_distance <= distance {
                far
            } else {
                value
            }
        }
        Self {
            x: axis(
                self.x,
                work.x,
                work.x.saturating_add(work.width.saturating_sub(self.width)),
                distance.max(0) as u32,
            ),
            y: axis(
                self.y,
                work.y,
                work.y
                    .saturating_add(work.height.saturating_sub(self.height)),
                distance.max(0) as u32,
            ),
            ..self
        }
    }

    pub fn fit(self, work: Self, minimum: [i32; 2]) -> Self {
        let width = self
            .width
            .clamp(minimum[0].min(work.width).max(1), work.width.max(1));
        let height = self
            .height
            .clamp(minimum[1].min(work.height).max(1), work.height.max(1));
        Self {
            x: self.x.clamp(
                work.x,
                work.x.saturating_add(work.width.saturating_sub(width)),
            ),
            y: self.y.clamp(
                work.y,
                work.y.saturating_add(work.height.saturating_sub(height)),
            ),
            width,
            height,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn snaps_to_each_edge_without_resizing() {
        let work = Geometry {
            x: 0,
            y: 40,
            width: 1920,
            height: 1000,
        };
        for (x, y, expected_x, expected_y) in [
            (16, 200, 0, 200),
            (1064, 200, 1080, 200),
            (300, 56, 300, 40),
            (300, 444, 300, 460),
        ] {
            let widget = Geometry {
                x,
                y,
                width: 840,
                height: 580,
            };
            assert_eq!(
                widget.snap(work, 16),
                Geometry {
                    x: expected_x,
                    y: expected_y,
                    ..widget
                }
            );
        }
    }

    #[test]
    fn dragging_away_releases_the_snap() {
        let work = Geometry {
            x: 0,
            y: 0,
            width: 1920,
            height: 1040,
        };
        // The drag uses the original grab offset, so moving beyond the snap zone
        // must leave the requested cursor-relative position unchanged.
        let widget = Geometry {
            x: 17,
            y: 17,
            width: 840,
            height: 580,
        };
        assert_eq!(widget.snap(work, 16), widget);
        let widget = Geometry {
            x: 1063,
            y: 443,
            ..widget
        };
        assert_eq!(widget.snap(work, 16), widget);
    }

    #[test]
    fn snaps_corners_on_negative_coordinate_monitors() {
        let work = Geometry {
            x: -1920,
            y: -400,
            width: 1920,
            height: 1040,
        };
        let widget = Geometry {
            x: -1908,
            y: -388,
            width: 840,
            height: 580,
        };
        assert_eq!(
            widget.snap(work, 16),
            Geometry {
                x: -1920,
                y: -400,
                ..widget
            }
        );
        let widget = Geometry {
            x: -852,
            y: 48,
            ..widget
        };
        assert_eq!(
            widget.snap(work, 16),
            Geometry {
                x: -840,
                y: 60,
                ..widget
            }
        );
    }

    #[test]
    fn removed_monitor_and_oversized_widget_stay_reachable() {
        let work = Geometry {
            x: 0,
            y: 40,
            width: 1920,
            height: 1000,
        };
        let saved = Geometry {
            x: -3000,
            y: -900,
            width: 4000,
            height: 2000,
        };
        assert_eq!(saved.fit(work, [700, 500]), work);
        let tiny = Geometry {
            x: 1900,
            y: 1020,
            width: 40,
            height: 20,
        };
        assert_eq!(
            tiny.fit(work, [700, 500]),
            Geometry {
                x: 1220,
                y: 540,
                width: 700,
                height: 500
            }
        );
    }
    #[test]
    fn negative_monitor_coordinates_are_preserved() {
        let saved = Geometry {
            x: -1500,
            y: -100,
            width: 840,
            height: 580,
        };
        let work = Geometry {
            x: -1920,
            y: -400,
            width: 1920,
            height: 1040,
        };
        assert_eq!(saved.fit(work, [700, 500]), saved);
    }
    #[test]
    fn small_work_area_takes_priority_over_minimum() {
        let work = Geometry {
            x: 0,
            y: 0,
            width: 600,
            height: 400,
        };
        assert_eq!(
            Geometry {
                x: 100,
                y: 100,
                width: 840,
                height: 580
            }
            .fit(work, [700, 500]),
            work
        );
    }
}
