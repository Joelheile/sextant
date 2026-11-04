//! Screen text from the Accessibility tree: exact text with roles, read before any
//! pixel. The platform reads one element at a time through [`AxSource`]; the walk, its budget and
//! what counts as content live here. OCR is the fallback when the tree says too little.

use crate::model::OcrLine;

/// Elements read per window at most. A long web page has more, but not on screen.
pub const MAX_NODES: usize = 3000;
const MAX_DEPTH: usize = 64;
/// Characters kept from one text element: editors hand over the whole document.
const MAX_NODE_CHARS: usize = 4000;
/// Buttons with shorter labels are toolbar words like "Reply" or "Send".
const MIN_BUTTON_LABEL: usize = 12;
/// Below this much text OCR reads the window instead: canvases, images, remote desktops.
const ENOUGH_CHARS: usize = 80;

/// Containers whose whole subtree is chrome, or must never be read.
const PRUNED: [&str; 10] = [
    "AXMenuBar",
    "AXMenu",
    "AXToolbar",
    "AXScrollBar",
    "AXSplitter",
    "AXRuler",
    "AXGrowArea",
    "AXBusyIndicator",
    "AXProgressIndicator",
    "AXSecureTextField",
];
const TEXT: [&str; 5] = [
    "AXStaticText",
    "AXTextField",
    "AXTextArea",
    "AXComboBox",
    "AXHeading",
];
const LABELLED: [&str; 6] = [
    "AXButton",
    "AXLink",
    "AXMenuButton",
    "AXPopUpButton",
    "AXRadioButton",
    "AXDisclosureTriangle",
];

/// One element as the OS reports it. `frame` is x, y, width, height in screen points with the
/// origin at the top left, `None` when the element has no position.
#[derive(Debug, Clone, Default)]
pub struct AxRead<E> {
    pub role: String,
    pub subrole: String,
    /// AXValue as text: the string of a text element, "0" or "1" for a checkbox.
    pub value: String,
    pub title: String,
    pub description: String,
    pub frame: Option<[f64; 4]>,
    pub children: Vec<E>,
}

pub trait AxSource {
    type Element;
    /// `None` when the element does not answer.
    fn read(&mut self, element: &Self::Element) -> Option<AxRead<Self::Element>>;
    /// Called before each read: true stops the walk with what was read so far.
    fn expired(&mut self) -> bool;
}

/// Whether the walk goes below an element: never into chrome or secure fields, never into
/// something entirely outside the window.
fn descend(role: &str, subrole: &str, frame: Option<[f32; 4]>) -> bool {
    !PRUNED.contains(&role) && subrole != "AXSecureTextField" && frame.is_none_or(visible)
}

fn visible(frame: [f32; 4]) -> bool {
    let [x, y, w, h] = frame;
    x < 1.0 && y < 1.0 && x + w > 0.0 && y + h > 0.0
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Check {
    Off,
    On,
    Mixed,
}

impl Check {
    fn mark(self) -> &'static str {
        match self {
            Self::Off => "[ ]",
            Self::On => "[x]",
            Self::Mixed => "[-]",
        }
    }
}

struct Piece {
    text: String,
    frame: [f32; 4],
    /// Index of the nearest AXRow above, so a row's cells become one line.
    row: Option<usize>,
    check: Option<Check>,
}

/// The visible text of the window `root`, as lines on the 256 grid, top to bottom. Each row of a
/// list or table is one line, its parts joined with " · ". A checkbox becomes "[ ]", "[x]" or
/// "[-]" in front of its label or the text next to it.
pub fn window_lines<S: AxSource>(source: &mut S, root: S::Element) -> Vec<OcrLine> {
    let Some(first) = (!source.expired()).then(|| source.read(&root)).flatten() else {
        return Vec::new();
    };
    let Some(window) = first.frame.filter(|[_, _, w, h]| *w > 0.0 && *h > 0.0) else {
        return Vec::new();
    };
    let relative = |frame: [f64; 4]| -> [f32; 4] {
        [
            ((frame[0] - window[0]) / window[2]) as f32,
            ((frame[1] - window[1]) / window[3]) as f32,
            (frame[2] / window[2]) as f32,
            (frame[3] / window[3]) as f32,
        ]
    };

    let mut pieces: Vec<Piece> = Vec::new();
    // (element, depth, nearest row). Children go on in reverse, so they come off in order.
    let mut stack: Vec<(AxRead<S::Element>, usize, Option<usize>)> = vec![(first, 0, None)];
    let mut read = 1;
    let mut node = 0;
    while let Some((element, depth, row)) = stack.pop() {
        node += 1;
        let frame = element.frame.map(relative);
        let row = if element.role == "AXRow" {
            Some(node)
        } else {
            row
        };
        if let Some(piece) = piece(&element, frame, row) {
            pieces.push(piece);
        }
        if depth >= MAX_DEPTH || !descend(&element.role, &element.subrole, frame) {
            continue;
        }
        let mut children = Vec::new();
        for child in &element.children {
            if read >= MAX_NODES || source.expired() {
                break;
            }
            read += 1;
            children.extend(source.read(child));
        }
        stack.extend(children.into_iter().rev().map(|c| (c, depth + 1, row)));
    }
    assemble(pieces)
}

fn piece<E>(element: &AxRead<E>, frame: Option<[f32; 4]>, row: Option<usize>) -> Option<Piece> {
    let role = element.role.as_str();
    if frame.is_some_and(|f| !visible(f)) || !descend(role, &element.subrole, None) {
        return None;
    }
    let label = || first_of(&[&element.title, &element.description]);
    let (text, check) = if role == "AXCheckBox" {
        let check = match element.value.as_str() {
            "1" => Check::On,
            "2" => Check::Mixed,
            _ => Check::Off,
        };
        (label(), Some(check))
    } else if TEXT.contains(&role) {
        (
            first_of(&[&element.value, &element.title, &element.description]),
            None,
        )
    } else if LABELLED.contains(&role) {
        let text = label();
        if text.chars().count() < MIN_BUTTON_LABEL {
            return None;
        }
        (text, None)
    } else {
        return None;
    };
    let text = clean(&text);
    if check.is_none() && !text.chars().any(char::is_alphanumeric) {
        return None;
    }
    Some(Piece {
        text,
        frame: frame.unwrap_or([0.0; 4]),
        row,
        check,
    })
}

fn first_of(options: &[&String]) -> String {
    options
        .iter()
        .find(|s| !s.trim().is_empty())
        .map_or_else(String::new, |s| s.to_string())
}

/// Collapses whitespace within each line and keeps the first lines of very long text.
fn clean(text: &str) -> String {
    let cut: String = text.chars().take(MAX_NODE_CHARS).collect();
    cut.lines()
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Joins rows, puts unlabelled checkboxes in front of the next text, drops a line that repeats the
/// one before it (a heading and its own text), and sorts.
fn assemble(pieces: Vec<Piece>) -> Vec<OcrLine> {
    let mut lines: Vec<(String, [f32; 4], Option<usize>)> = Vec::new();
    let mut pending: Option<(Check, [f32; 4], Option<usize>)> = None;
    for piece in pieces {
        let mut text = piece.text;
        let mut frame = piece.frame;
        if let Some(check) = piece.check {
            if text.is_empty() {
                pending = Some((check, frame, piece.row));
                continue;
            }
            text = format!("{} {text}", check.mark());
        } else if let Some((check, at, row)) = pending.take()
            && same_row(row, piece.row, at, frame)
        {
            text = format!("{} {text}", check.mark());
            frame = union(at, frame);
        }
        match lines.last_mut() {
            Some(last) if last.0 == text => {}
            Some(last) if last.2.is_some() && last.2 == piece.row => {
                last.0 = format!("{} · {text}", last.0);
                last.1 = union(last.1, frame);
            }
            _ => lines.push((text, frame, piece.row)),
        }
    }
    let mut out: Vec<OcrLine> = lines
        .into_iter()
        .flat_map(|(text, frame, _)| {
            let bbox = frame.map(cell);
            text.lines()
                .map(|line| OcrLine {
                    text: line.to_string(),
                    bbox,
                })
                .collect::<Vec<_>>()
        })
        .collect();
    out.sort_by_key(|line| (line.bbox[1], line.bbox[0]));
    out
}

/// In the same table row, or on the same visual line.
fn same_row(a_row: Option<usize>, b_row: Option<usize>, a: [f32; 4], b: [f32; 4]) -> bool {
    if a_row.is_some() && a_row == b_row {
        return true;
    }
    let (a_mid, b_mid) = (a[1] + a[3] / 2.0, b[1] + b[3] / 2.0);
    (a_mid - b_mid).abs() <= a[3].max(b[3]) / 2.0
}

fn union(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    if a == [0.0; 4] {
        return b;
    }
    if b == [0.0; 4] {
        return a;
    }
    let (x, y) = (a[0].min(b[0]), a[1].min(b[1]));
    let right = (a[0] + a[2]).max(b[0] + b[2]);
    let bottom = (a[1] + a[3]).max(b[1] + b[3]);
    [x, y, right - x, bottom - y]
}

/// A fraction of the window on the 0..=255 grid, as for OCR boxes.
fn cell(fraction: f32) -> u8 {
    (fraction * 256.0).floor().clamp(0.0, 255.0) as u8
}

/// Whether the tree said enough to skip the screenshot and OCR.
pub fn enough(lines: &[OcrLine]) -> bool {
    lines
        .iter()
        .flat_map(|line| line.text.chars())
        .filter(|c| c.is_alphanumeric())
        .count()
        >= ENOUGH_CHARS
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tree held in a vector; elements are indexes.
    struct Tree {
        nodes: Vec<AxRead<usize>>,
        reads: usize,
        budget: usize,
    }

    impl Tree {
        fn new() -> Self {
            let mut tree = Self {
                nodes: Vec::new(),
                reads: 0,
                budget: usize::MAX,
            };
            tree.add(None, "AXWindow", "", [100.0, 100.0, 1000.0, 800.0]);
            tree
        }

        fn add(&mut self, parent: Option<usize>, role: &str, text: &str, frame: [f64; 4]) -> usize {
            let index = self.nodes.len();
            self.nodes.push(AxRead {
                role: role.into(),
                value: text.into(),
                frame: Some(frame),
                ..AxRead::default()
            });
            if let Some(parent) = parent {
                self.nodes[parent].children.push(index);
            }
            index
        }

        /// A line of text at window row `y` (points from the window top).
        fn text(&mut self, parent: usize, text: &str, y: f64) -> usize {
            self.add(
                Some(parent),
                "AXStaticText",
                text,
                [120.0, 100.0 + y, 400.0, 20.0],
            )
        }

        fn lines(&mut self) -> Vec<String> {
            window_lines(self, 0).into_iter().map(|l| l.text).collect()
        }
    }

    impl AxSource for Tree {
        type Element = usize;

        fn read(&mut self, element: &usize) -> Option<AxRead<usize>> {
            self.reads += 1;
            self.nodes.get(*element).cloned()
        }

        fn expired(&mut self) -> bool {
            self.reads >= self.budget
        }
    }

    #[test]
    fn text_comes_out_top_to_bottom_without_chrome() {
        let mut tree = Tree::new();
        let toolbar = tree.add(Some(0), "AXToolbar", "", [100.0, 100.0, 1000.0, 40.0]);
        tree.add(
            Some(toolbar),
            "AXButton",
            "Archive this conversation",
            [110.0, 105.0, 80.0, 30.0],
        );
        tree.text(0, "Total 1.250,00 €", 300.0);
        tree.text(0, "Invoice   INV-2026-0412", 200.0);
        tree.add(Some(0), "AXButton", "Reply", [120.0, 500.0, 60.0, 20.0]);
        assert_eq!(tree.lines(), ["Invoice INV-2026-0412", "Total 1.250,00 €"]);
    }

    #[test]
    fn secure_fields_and_their_children_are_never_read() {
        let mut tree = Tree::new();
        let secret = tree.add(
            Some(0),
            "AXSecureTextField",
            "hunter2",
            [120.0, 200.0, 200.0, 20.0],
        );
        tree.text(secret, "hunter2", 0.0);
        let field = tree.add(Some(0), "AXTextField", "pin", [120.0, 300.0, 200.0, 20.0]);
        tree.nodes[field].subrole = "AXSecureTextField".into();
        tree.text(0, "Sign in", 400.0);
        assert_eq!(tree.lines(), ["Sign in"]);
    }

    #[test]
    fn offscreen_subtrees_are_skipped() {
        let mut tree = Tree::new();
        let below = tree.add(Some(0), "AXGroup", "", [100.0, 2000.0, 1000.0, 800.0]);
        tree.add(
            Some(below),
            "AXStaticText",
            "far below",
            [120.0, 2100.0, 400.0, 20.0],
        );
        tree.text(0, "on screen", 100.0);
        assert_eq!(tree.lines(), ["on screen"]);
        assert_eq!(tree.reads, 3, "the offscreen child is never read");
    }

    #[test]
    fn a_row_becomes_one_line_and_checkboxes_keep_their_state() {
        let mut tree = Tree::new();
        let list = tree.add(Some(0), "AXOutline", "", [100.0, 100.0, 1000.0, 800.0]);
        for (y, done, title, due) in [
            (200.0, "0", "Pay Acme invoice", "Tomorrow"),
            (240.0, "1", "Send offer", "Today"),
        ] {
            let row = tree.add(Some(list), "AXRow", "", [100.0, 100.0 + y, 1000.0, 30.0]);
            let check = tree.add(
                Some(row),
                "AXCheckBox",
                done,
                [110.0, 100.0 + y, 20.0, 20.0],
            );
            tree.nodes[check].value = done.into();
            tree.add(
                Some(row),
                "AXTextField",
                title,
                [140.0, 100.0 + y, 300.0, 20.0],
            );
            tree.add(
                Some(row),
                "AXStaticText",
                due,
                [800.0, 100.0 + y, 100.0, 20.0],
            );
        }
        assert_eq!(
            tree.lines(),
            ["[ ] Pay Acme invoice · Tomorrow", "[x] Send offer · Today"]
        );
    }

    #[test]
    fn a_web_checkbox_takes_the_label_beside_it() {
        let mut tree = Tree::new();
        let web = tree.add(Some(0), "AXWebArea", "", [100.0, 100.0, 1000.0, 800.0]);
        let check = tree.add(Some(web), "AXCheckBox", "1", [110.0, 300.0, 20.0, 20.0]);
        tree.nodes[check].value = "1".into();
        tree.text(web, "Draft the Q3 report", 200.0);
        tree.text(web, "Unrelated footer", 600.0);
        assert_eq!(
            tree.lines(),
            ["[x] Draft the Q3 report", "Unrelated footer"]
        );
    }

    #[test]
    fn long_button_labels_are_content_like_calendar_events() {
        let mut tree = Tree::new();
        let event = tree.add(Some(0), "AXButton", "", [120.0, 300.0, 200.0, 40.0]);
        tree.nodes[event].title = "10am to 11am, Weekly sync, Accepted".into();
        assert_eq!(tree.lines(), ["10am to 11am, Weekly sync, Accepted"]);
    }

    #[test]
    fn a_heading_repeated_by_its_text_child_appears_once() {
        let mut tree = Tree::new();
        let heading = tree.add(
            Some(0),
            "AXHeading",
            "Rechnung bearbeiten",
            [120.0, 200.0, 400.0, 30.0],
        );
        tree.text(heading, "Rechnung bearbeiten", 100.0);
        assert_eq!(tree.lines(), ["Rechnung bearbeiten"]);
    }

    #[test]
    fn the_walk_stops_when_time_runs_out() {
        let mut tree = Tree::new();
        for y in 0..10 {
            tree.text(0, &format!("line {y}"), 50.0 * y as f64);
        }
        tree.budget = 4;
        assert_eq!(tree.lines().len(), 3, "root plus three children were read");
    }

    #[test]
    fn a_window_without_a_frame_gives_nothing() {
        let mut tree = Tree::new();
        tree.text(0, "text", 10.0);
        tree.nodes[0].frame = None;
        assert!(tree.lines().is_empty());
    }

    #[test]
    fn little_text_falls_back_to_ocr() {
        let line = |text: &str| OcrLine {
            text: text.into(),
            bbox: [0; 4],
        };
        assert!(!enough(&[line("Untitled"), line("Frame 1")]));
        assert!(enough(&[line(&"word ".repeat(20))]));
    }
}
