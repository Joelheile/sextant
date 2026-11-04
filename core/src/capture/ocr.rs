//! What to keep from an OCR result: lines with enough confidence, boxes on the
//! 256 grid of `frames.packed`.

use crate::model::OcrLine;

/// Lines the recognizer is less sure about than this are dropped.
pub const MIN_CONFIDENCE: f32 = 0.3;

/// One recognized line as the OS reports it. Box values are fractions of the window, with the
/// origin at the top left.
pub struct RawLine {
    pub text: String,
    pub confidence: f32,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// Drops low-confidence and empty lines, puts the boxes on the grid and sorts top to bottom, so
/// the same screen gives the same list and the writer can dedupe it.
pub fn lines(raw: Vec<RawLine>) -> Vec<OcrLine> {
    let mut kept: Vec<OcrLine> = raw
        .into_iter()
        .filter(|line| line.confidence >= MIN_CONFIDENCE)
        .filter_map(|line| {
            let text = line.text.trim();
            (!text.is_empty()).then(|| OcrLine {
                text: text.to_string(),
                bbox: [
                    cell(line.x),
                    cell(line.y),
                    cell(line.width),
                    cell(line.height),
                ],
            })
        })
        .collect();
    kept.sort_by_key(|line| (line.bbox[1], line.bbox[0]));
    kept
}

/// A fraction of the window on the 0..=255 grid. Values outside the window are clamped.
fn cell(fraction: f32) -> u8 {
    (fraction * 256.0).floor().clamp(0.0, 255.0) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(text: &str, confidence: f32, x: f32, y: f32) -> RawLine {
        RawLine {
            text: text.into(),
            confidence,
            x,
            y,
            width: 0.5,
            height: 0.02,
        }
    }

    #[test]
    fn low_confidence_and_empty_lines_are_dropped() {
        let kept = lines(vec![
            raw("Invoice 4711", 0.9, 0.1, 0.1),
            raw("l1lI", 0.29, 0.1, 0.2),
            raw("Total", 0.3, 0.1, 0.3),
            raw("   ", 0.9, 0.1, 0.4),
        ]);
        let text: Vec<&str> = kept.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(text, ["Invoice 4711", "Total"]);
    }

    #[test]
    fn boxes_land_on_the_256_grid() {
        let kept = lines(vec![RawLine {
            text: "x".into(),
            confidence: 1.0,
            x: 0.5,
            y: 1.0,
            width: 1.2,
            height: -0.1,
        }]);
        assert_eq!(kept[0].bbox, [128, 255, 255, 0]);
    }

    #[test]
    fn lines_come_out_top_to_bottom_then_left_to_right() {
        let kept = lines(vec![
            raw("c", 0.9, 0.1, 0.8),
            raw("b", 0.9, 0.6, 0.1),
            raw("a", 0.9, 0.1, 0.1),
        ]);
        let text: Vec<&str> = kept.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(text, ["a", "b", "c"]);
    }
}
