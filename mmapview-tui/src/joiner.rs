use crate::app::OVERSCAN_STEPS;
use crate::overscan::Overscan;
use crate::scanner::Scanner;
use crate::viewport::Viewport;
use mmapstruct_schema::SchemaValue;

/// Top-down joiner for stitching two Overscan sections together with overlap.
pub struct TopDownJoiner;

impl TopDownJoiner {
    /// Joins two Overscan sections with overlap for seamless navigation.
    ///
    /// Creates a new Overscan that starts from `scan_start` (computed by
    /// stepping back from `prev_end` by `overlap_lines`), ensuring the
    /// overlap region is included for UI "rewind" capability.
    pub fn join(
        prev_overscan: &Overscan,
        overlap_lines: usize,
        remaining: usize,
        display_height: usize,
    ) -> (Overscan, Viewport) {
        let prev_end = prev_overscan.effective_end().unwrap();
        let ctx = prev_overscan.scanner.ctx;

        let back_scanner = Scanner::from_path(ctx, prev_end.clone(), 0);
        let should_step_in = |val: SchemaValue| prev_overscan.is_expanded(&val);

        let (scan_start, _) = back_scanner.do_steps_back(&prev_end, overlap_lines, &should_step_in);

        let new_scanner = Scanner::from_path(ctx, scan_start.clone(), OVERSCAN_STEPS);

        let selected = overlap_lines + (remaining - 1);
        let scroll = selected - (display_height - 1);

        (
            Overscan::from_previous(prev_overscan, new_scanner),
            Viewport {
                selected,
                scroll,
                display_height,
            },
        )
    }
}

/// Bottom-up joiner for stitching two Overscan sections together with overlap
/// when scrolling upward.
pub struct BottomUpJoiner;

impl BottomUpJoiner {
    /// Joins two Overscan sections when scrolling up.
    ///
    /// Steps back from the current Overscan's begin by `overlap_lines + remaining`
    /// to compute `scan_start`, ensuring the overlap region covers the tail of
    /// the new upper Overscan.
    pub fn join(
        prev_overscan: &Overscan,
        overlap_lines: usize,
        remaining: usize,
        display_height: usize,
    ) -> (Overscan, Viewport) {
        let begin = prev_overscan.scanner.path.clone();
        let ctx = prev_overscan.scanner.ctx;

        let step_back = overlap_lines + remaining;
        let back_scanner = Scanner::from_path(ctx, begin.clone(), 0);
        let should_step_in = |val: SchemaValue| prev_overscan.is_expanded(&val);

        let (scan_start, steps) = back_scanner.do_steps_back(&begin, step_back, &should_step_in);

        let new_scanner = Scanner::from_path(ctx, scan_start.clone(), OVERSCAN_STEPS);

        let (selected, scroll) = if steps < remaining {
            // Not enough data above — can't jump as far as wanted
            (0, 0)
        } else {
            let overlap_actual = steps - remaining; // lines after remaining that form the overlap
            let sel = overlap_lines.min(overlap_actual);
            (sel, sel)
        };

        (
            Overscan::from_next(prev_overscan, new_scanner),
            Viewport {
                selected,
                scroll,
                display_height,
            },
        )
    }
}
