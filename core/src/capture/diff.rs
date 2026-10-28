//! Diff gate (plan.md §4): a 64×64 grayscale thumbnail of every frame, compared with the last
//! frame of the same window. A frame that is nearly the same skips OCR.
//!
//! The reference only moves when a frame counts as changed, so small edits that are each below
//! the threshold add up until one frame passes.

use std::collections::{HashMap, VecDeque};
use std::fmt;

pub const SIDE: usize = 64;
/// A cell counts as changed when its brightness moves by at least this much (of 255). A text
/// caret moves one cell by about 26 at 1440×900, a typed word moves a few cells by 40 or more.
pub const CELL_DELTA: u8 = 30;
/// Changed cells needed for a frame to count as changed. A caret straddling two cells stays below.
pub const MIN_CELLS: usize = 3;
/// Windows whose last thumbnail is kept.
const WINDOWS: usize = 32;

#[derive(Clone, PartialEq, Eq)]
pub struct Thumb(Box<[u8; SIDE * SIDE]>);

/// Pixels never show up in logs or panic messages: only the size.
impl fmt::Debug for Thumb {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Thumb({SIDE}x{SIDE})")
    }
}

impl Thumb {
    /// Area average of a row-major grayscale buffer down to 64×64. `None` when the buffer does
    /// not match its size or one side is zero.
    pub fn from_gray(width: usize, height: usize, pixels: &[u8]) -> Option<Self> {
        if width == 0 || height == 0 || pixels.len() != width * height {
            return None;
        }
        let mut cells = Box::new([0u8; SIDE * SIDE]);
        for cy in 0..SIDE {
            let (y0, y1) = span(cy, height);
            for cx in 0..SIDE {
                let (x0, x1) = span(cx, width);
                let mut sum = 0u64;
                for y in y0..y1 {
                    sum += pixels[y * width + x0..y * width + x1]
                        .iter()
                        .map(|&p| u64::from(p))
                        .sum::<u64>();
                }
                let count = ((y1 - y0) * (x1 - x0)) as u64;
                cells[cy * SIDE + cx] = (sum / count) as u8;
            }
        }
        Some(Self(cells))
    }

    fn changed_cells(&self, other: &Thumb) -> usize {
        self.0
            .iter()
            .zip(other.0.iter())
            .filter(|(a, b)| a.abs_diff(**b) >= CELL_DELTA)
            .count()
    }
}

/// Pixel range of cell `i` out of 64 along a side of `len` pixels. Never empty.
fn span(i: usize, len: usize) -> (usize, usize) {
    let start = i * len / SIDE;
    let end = ((i + 1) * len / SIDE).max(start + 1).min(len);
    (start.min(end - 1), end)
}

#[derive(Default)]
pub struct DiffGate {
    last: HashMap<u64, Thumb>,
    order: VecDeque<u64>,
}

impl DiffGate {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the frame of `window` differs enough from the last frame that passed. A window
    /// seen for the first time always passes.
    pub fn changed(&mut self, window: u64, thumb: Thumb) -> bool {
        if let Some(last) = self.last.get(&window)
            && last.changed_cells(&thumb) < MIN_CELLS
        {
            return false;
        }
        if self.last.insert(window, thumb).is_none() {
            self.order.push_back(window);
            if self.order.len() > WINDOWS
                && let Some(oldest) = self.order.pop_front()
            {
                self.last.remove(&oldest);
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::Rng;

    /// Synthetic window: white page with lines of text-like stripes (dark 2 px, light 6 px, so
    /// a quarter of the pixels are ink), the shape that makes a 64×64 thumbnail move when text
    /// does.
    struct Page {
        width: usize,
        height: usize,
        pixels: Vec<u8>,
    }

    impl Page {
        fn new(width: usize, height: usize) -> Self {
            Self {
                width,
                height,
                pixels: vec![255; width * height],
            }
        }

        /// Fills a rectangle with a constant gray.
        fn rect(&mut self, x: usize, y: usize, w: usize, h: usize, gray: u8) {
            for row in y..(y + h).min(self.height) {
                for col in x..(x + w).min(self.width) {
                    self.pixels[row * self.width + col] = gray;
                }
            }
        }

        /// A run of text: `h` pixels tall, stripes of ink.
        fn text(&mut self, x: usize, y: usize, w: usize, h: usize) {
            let scale = self.width / 1440;
            for col in (x..x + w).step_by(8 * scale) {
                self.rect(col, y, 2 * scale, h, 40);
            }
        }

        /// `lines` rows of text at 24 px spacing, every row filled with words of random length.
        fn document(width: usize, height: usize, seed: u64) -> Self {
            let mut page = Self::new(width, height);
            let scale = width / 1440;
            let mut rng = Rng(seed);
            for line in 0..30 {
                let y = (60 + line * 24) * scale;
                let mut x = 80 * scale;
                while x < 1300 * scale {
                    let w = (30 + rng.below(60) as usize) * scale;
                    page.text(x, y, w.min(1300 * scale - x), 14 * scale);
                    x += w + 12 * scale;
                }
            }
            page
        }

        fn thumb(&self) -> Thumb {
            Thumb::from_gray(self.width, self.height, &self.pixels).expect("sized buffer")
        }
    }

    /// Both sizes the capture produces: points and Retina pixels.
    const SIZES: [(usize, usize); 2] = [(1440, 900), (2880, 1800)];

    fn gate_with(page: &Page) -> DiffGate {
        let mut gate = DiffGate::new();
        assert!(gate.changed(1, page.thumb()), "the first frame passes");
        gate
    }

    #[test]
    fn an_identical_frame_is_skipped() {
        for (w, h) in SIZES {
            let page = Page::document(w, h, 1);
            let mut gate = gate_with(&page);
            assert!(!gate.changed(1, page.thumb()));
        }
    }

    #[test]
    fn a_blinking_caret_is_skipped() {
        for (w, h) in SIZES {
            let page = Page::document(w, h, 2);
            let mut gate = gate_with(&page);
            let scale = w / 1440;
            for (x, y) in [(500, 300), (503, 301), (511, 500), (45, 700)] {
                let mut with_caret = Page::document(w, h, 2);
                with_caret.rect(x * scale, y * scale, 2 * scale, 16 * scale, 0);
                assert!(!gate.changed(1, with_caret.thumb()), "caret at {x},{y}");
            }
        }
    }

    #[test]
    fn compression_noise_is_skipped() {
        for (w, h) in SIZES {
            let page = Page::document(w, h, 3);
            let mut gate = gate_with(&page);
            let mut rng = Rng(9);
            let mut noisy = Page::document(w, h, 3);
            for pixel in &mut noisy.pixels {
                let jitter = rng.below(5) as i16 - 2;
                *pixel = (i16::from(*pixel) + jitter).clamp(0, 255) as u8;
            }
            assert!(!gate.changed(1, noisy.thumb()));
        }
    }

    #[test]
    fn a_typed_sentence_passes() {
        for (w, h) in SIZES {
            let page = Page::document(w, h, 4);
            let mut gate = gate_with(&page);
            let scale = w / 1440;
            let mut typed = Page::document(w, h, 4);
            typed.rect(80 * scale, 790 * scale, 400 * scale, 14 * scale, 255);
            typed.text(80 * scale, 790 * scale, 400 * scale, 14 * scale);
            assert!(gate.changed(1, typed.thumb()));
        }
    }

    #[test]
    fn scrolling_by_a_few_lines_passes() {
        for (w, h) in SIZES {
            let page = Page::document(w, h, 5);
            let mut gate = gate_with(&page);
            let scale = w / 1440;
            let shift = 3 * 24 * scale;
            let mut scrolled = Page::new(w, h);
            let rows = h - shift;
            scrolled.pixels[..rows * w].copy_from_slice(&page.pixels[shift * w..]);
            assert!(gate.changed(1, scrolled.thumb()));
        }
    }

    #[test]
    fn another_document_in_the_same_window_passes() {
        for (w, h) in SIZES {
            let mut gate = gate_with(&Page::document(w, h, 6));
            assert!(gate.changed(1, Page::document(w, h, 7).thumb()));
        }
    }

    #[test]
    fn small_edits_add_up_because_the_reference_stays_put() {
        let (w, h) = (1440, 900);
        let mut gate = gate_with(&Page::document(w, h, 8));
        let mut page = Page::document(w, h, 8);
        let mut passed_at = None;
        for word in 0..12 {
            page.text(100 + word * 100, 800, 40, 14);
            if gate.changed(1, page.thumb()) {
                passed_at = Some(word);
                break;
            }
        }
        let word = passed_at.expect("a dozen words eventually pass");
        assert!(word > 0, "a single short word stays below the threshold");
    }

    #[test]
    fn windows_are_compared_only_with_themselves() {
        let page = Page::document(1440, 900, 10);
        let mut gate = gate_with(&page);
        assert!(
            gate.changed(2, page.thumb()),
            "window 2 has no reference yet"
        );
        assert!(!gate.changed(2, page.thumb()));
        assert!(!gate.changed(1, page.thumb()));
    }

    #[test]
    fn thumbnails_print_their_size_and_nothing_else() {
        let page = Page::document(1440, 900, 11);
        assert_eq!(format!("{:?}", page.thumb()), "Thumb(64x64)");
        assert!(Thumb::from_gray(10, 10, &[0; 99]).is_none());
        assert!(Thumb::from_gray(0, 10, &[]).is_none());
    }

    #[test]
    fn small_buffers_scale_up_without_empty_cells() {
        let thumb = Thumb::from_gray(8, 8, &[7; 64]).expect("sized buffer");
        assert!(thumb.0.iter().all(|&c| c == 7));
    }

    #[test]
    fn old_windows_are_forgotten_beyond_the_cap() {
        let page = Page::document(1440, 900, 12);
        let mut gate = DiffGate::new();
        for window in 0..=WINDOWS as u64 {
            assert!(gate.changed(window, page.thumb()));
        }
        assert!(gate.changed(0, page.thumb()), "window 0 was evicted");
        assert!(!gate.changed(WINDOWS as u64, page.thumb()));
    }
}
