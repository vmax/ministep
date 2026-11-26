//! Window geometry: every fixed size in one place, and the region split
//! computed from the window rect (unit-tested for overlaps).

use eframe::egui::{Rect, pos2, vec2};

/// Steps shown per grid page (and the drum bar length).
pub const STEPS_PER_PAGE: usize = 16;

// --- window frame ---------------------------------------------------------------------------
/// Side margin; also clears the macOS traffic lights with the full-size content view.
pub const SIDE_MARGIN: f32 = 28.0;
pub const TOP_BAR_OFFSET: f32 = 26.0;
pub const TOP_BAR_HEIGHT: f32 = 76.0;
pub const STATUS_HEIGHT: f32 = 22.0;
pub const STATUS_BOTTOM_MARGIN: f32 = 8.0;
pub const BODY_GAP: f32 = 28.0;

// --- top bar ---------------------------------------------------------------------------
/// Logo + pattern name.
pub const TITLE_WIDTH: f32 = 320.0;
/// Theme, patterns and settings icons.
pub const RIGHT_ICONS_WIDTH: f32 = 190.0;
/// From the stop button's left edge to the end of the gate readout.
pub const TRANSPORT_CLUSTER_WIDTH: f32 = 640.0;
/// Play button centre, measured from the cluster's left edge.
pub const TRANSPORT_MID_OFFSET: f32 = 122.0;

// --- columns --------------------------------------------------------------------------------
pub const TRACK_COLUMN_WIDTH: f32 = 300.0;
pub const COLUMN_GAP: f32 = 8.0;
/// The track list starts below the "TRACK" heading row.
pub const TRACK_LIST_OFFSET: f32 = 64.0;
pub const TRACK_ROW_HEIGHT: f32 = 52.0;
/// Drum lanes under the "drums" group entry.
pub const LANE_ROW_HEIGHT: f32 = 40.0;
pub const LANE_INDENT: f32 = 22.0;
pub const ADD_BUTTON_HEIGHT: f32 = 48.0;

// --- step grid ------------------------------------------------------------------------------
pub const TOOLS_HEIGHT: f32 = 52.0;
/// Space between the tools row and the ruler.
pub const TOOLS_TO_GRID: f32 = 84.0;
/// Left gutter of the grid (commit dot, "VEL" label).
pub const GUTTER: f32 = 56.0;
pub const CELL_GAP: f32 = 8.0;
pub const CELL_MIN_HEIGHT: f32 = 48.0;
pub const CELL_MAX_HEIGHT: f32 = 96.0;
pub const CELL_RADIUS: f32 = 10.0;
pub const VEL_MIN_HEIGHT: f32 = 80.0;
pub const VEL_MAX_HEIGHT: f32 = 150.0;
pub const LOOP_PILL_WIDTH: f32 = 96.0;

// --- drum group grid ------------------------------------------------------------------------
/// Wider gutter in the drum grid: it holds the lane names.
pub const DRUM_GUTTER: f32 = 104.0;
pub const DRUM_ROW_MIN: f32 = 14.0;
pub const DRUM_VEL_GAP: f32 = 14.0;

/// Geometry of the drum grid below the ruler.
#[derive(Debug, Clone, Copy)]
pub struct DrumRows {
    pub row: f32,
    pub gap: f32,
    pub vel: f32,
}

/// Fit `lanes` rows plus the velocity lane into `height` pixels. With many
/// lanes the velocity lane and the gaps shrink before the rows do.
pub fn drum_rows(height: f32, lanes: usize) -> DrumRows {
    let n = lanes.max(1);
    let (gap, vel) = if n <= 5 { (6.0, 64.0) } else { (4.0, 44.0) };
    let rows = height - DRUM_VEL_GAP - vel;
    let row = (rows / n as f32 - gap).clamp(DRUM_ROW_MIN, CELL_MAX_HEIGHT * 0.75);
    DrumRows { row, gap, vel }
}

// --- bottom strip ---------------------------------------------------------------------------
/// The strip takes this share of the window height, within the bounds below.
pub const STRIP_SHARE: f32 = 0.3;
pub const STRIP_MIN_HEIGHT: f32 = 240.0;
pub const STRIP_MAX_HEIGHT: f32 = 300.0;
/// Gap kept between the step grid and the strip.
pub const STRIP_GAP: f32 = 30.0;
pub const CARD_WIDTH: f32 = 300.0;
pub const CARD_TO_PANEL: f32 = 56.0;
pub const KNOB_WIDTH: f32 = 104.0;
pub const KNOB_WIDTH_LARGE: f32 = 118.0;
pub const ENVELOPE_WIDTH: f32 = 330.0;
pub const ENVELOPE_WIDTH_LARGE: f32 = 380.0;
pub const CHIP_WIDTH: f32 = 62.0;
pub const CHIP_HEIGHT: f32 = 34.0;
pub const WAVE_WIDTH: f32 = 300.0;
pub const WAVE_MIN_WIDTH: f32 = 150.0;

/// Smallest window the layout is designed for (see the tests).
pub const MIN_WINDOW: [f32; 2] = [1280.0, 820.0];

/// The regions of the window for one frame.
#[derive(Debug, Clone, Copy)]
pub struct Layout {
    pub full: Rect,
    pub top: Rect,
    pub status: Rect,
    pub tracks: Rect,
    pub tools: Rect,
    /// Step tab: the grid.
    pub steps: Rect,
    /// Step tab: the bottom strip (track card + synth).
    pub strip: Rect,
    /// Synth tab body.
    pub synth: Rect,
}

impl Layout {
    pub fn new(full: Rect) -> Self {
        let area = full.shrink2(vec2(SIDE_MARGIN, 0.0));
        let top = Rect::from_min_size(pos2(area.left(), area.top() + TOP_BAR_OFFSET), vec2(area.width(), TOP_BAR_HEIGHT));
        let status = Rect::from_min_max(
            pos2(area.left(), full.bottom() - STATUS_BOTTOM_MARGIN - STATUS_HEIGHT),
            pos2(area.right(), full.bottom() - STATUS_BOTTOM_MARGIN),
        );
        let body_top = top.bottom() + BODY_GAP;
        let strip_top = status.top() - (full.height() * STRIP_SHARE).clamp(STRIP_MIN_HEIGHT, STRIP_MAX_HEIGHT);
        let tracks = Rect::from_min_max(
            pos2(area.left(), body_top + TRACK_LIST_OFFSET),
            pos2(area.left() + TRACK_COLUMN_WIDTH, strip_top),
        );
        let main = Rect::from_min_max(pos2(area.left() + TRACK_COLUMN_WIDTH + COLUMN_GAP, body_top), pos2(area.right(), status.top()));
        let tools = Rect::from_min_size(main.min, vec2(main.width(), TOOLS_HEIGHT));
        let grid_top = main.top() + TOOLS_TO_GRID;
        let steps = Rect::from_min_max(pos2(main.left(), grid_top), pos2(main.right(), strip_top - STRIP_GAP));
        let strip = Rect::from_min_max(pos2(area.left(), strip_top), pos2(area.right(), status.top() - STATUS_BOTTOM_MARGIN));
        let synth = Rect::from_min_max(pos2(main.left(), grid_top), pos2(main.right(), status.top() - STATUS_BOTTOM_MARGIN));
        Self { full, top, status, tracks, tools, steps, strip, synth }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn transport_cluster_fits_between_title_and_icons_at_minimum_width() {
        let free = MIN_WINDOW[0] - 2.0 * SIDE_MARGIN - TITLE_WIDTH - RIGHT_ICONS_WIDTH;
        assert!(free >= TRANSPORT_CLUSTER_WIDTH, "{free} < {TRANSPORT_CLUSTER_WIDTH}");
    }

    use super::*;

    fn disjoint(a: Rect, b: Rect) -> bool {
        a.max.x <= b.min.x + 0.5 || b.max.x <= a.min.x + 0.5 || a.max.y <= b.min.y + 0.5 || b.max.y <= a.min.y + 0.5
    }

    #[test]
    fn regions_do_not_overlap_from_minimum_to_large_windows() {
        for (w, h) in [(MIN_WINDOW[0], MIN_WINDOW[1]), (1440.0, 900.0), (1680.0, 1050.0), (2560.0, 1440.0)] {
            let l = Layout::new(Rect::from_min_size(pos2(0.0, 0.0), vec2(w, h)));
            let named = [("top", l.top), ("tracks", l.tracks), ("tools", l.tools), ("steps", l.steps), ("strip", l.strip), ("status", l.status)];
            for (i, (na, a)) in named.iter().enumerate() {
                assert!(l.full.contains_rect(*a), "{na} outside the window at {w}x{h}");
                assert!(a.width() > 0.0 && a.height() > 0.0, "{na} collapsed at {w}x{h}: {a:?}");
                for (nb, b) in &named[i + 1..] {
                    if *na == "tracks" && *nb == "tools" {
                        continue; // different columns by construction, checked below
                    }
                    assert!(disjoint(*a, *b), "{na} overlaps {nb} at {w}x{h}");
                }
            }
            assert!(l.tracks.right() < l.tools.left(), "columns overlap at {w}x{h}");
        }
    }

    #[test]
    fn drum_group_fits_up_to_eight_lanes_at_minimum_size() {
        let l = Layout::new(Rect::from_min_size(pos2(0.0, 0.0), vec2(MIN_WINDOW[0], MIN_WINDOW[1])));
        let below_ruler = l.steps.height() - 56.0;
        for lanes in 1..=8 {
            let g = drum_rows(below_ruler, lanes);
            let used = lanes as f32 * (g.row + g.gap) + DRUM_VEL_GAP + g.vel;
            assert!(g.row >= 16.0, "{lanes} lanes: rows only {}px", g.row);
            assert!(used <= below_ruler + 0.5, "{lanes} lanes overflow: {used} > {below_ruler}");
        }
    }

    #[test]
    fn grid_has_room_for_cells_and_velocity_at_minimum_size() {
        let l = Layout::new(Rect::from_min_size(pos2(0.0, 0.0), vec2(MIN_WINDOW[0], MIN_WINDOW[1])));
        // Ruler (~56) + cells + gap (22) + velocity lane.
        let needed = 56.0 + CELL_MIN_HEIGHT + 22.0 + VEL_MIN_HEIGHT;
        assert!(l.steps.height() >= needed, "grid {} < {needed}", l.steps.height());
        // Heading (38) + melodic + drums group, then the 4 default lanes and "+".
        let lanes = ((l.tracks.height() - 38.0 - 2.0 * TRACK_ROW_HEIGHT - ADD_BUTTON_HEIGHT) / LANE_ROW_HEIGHT).floor() as usize;
        assert!(lanes >= 4, "only {lanes} lane rows fit");
    }
}
