//! Synthetic window images for the OCR budget and the bench. Text is drawn
//! with AppKit into a bitmap. The real screen and real windows are never captured to make these.

use anyhow::{Context, Result};
use objc2::rc::autoreleasepool;
use objc2::runtime::AnyObject;
use objc2_app_kit::{
    NSColor, NSFont, NSFontAttributeName, NSFontWeightRegular, NSForegroundColorAttributeName,
    NSGraphicsContext, NSStringDrawing,
};
use objc2_core_foundation::{CFRetained, CGPoint, CGRect, CGSize};
use objc2_core_graphics::{
    CGBitmapContextCreate, CGBitmapContextCreateImage, CGColorSpace, CGContext, CGImageAlphaInfo,
};
use objc2_foundation::{NSDictionary, NSString};

use super::vision::Frame;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Email,
    Spreadsheet,
    Invoice,
    Editor,
}

pub const KINDS: [Kind; 4] = [Kind::Email, Kind::Spreadsheet, Kind::Invoice, Kind::Editor];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    English,
    German,
}

pub const LANGUAGES: [Language; 2] = [Language::English, Language::German];

/// The window sizes of the budget: a 1440x900 display and the same display at 2x.
pub const SIZES: [(usize, usize); 2] = [(1440, 900), (2880, 1800)];

pub struct Fixture {
    pub name: String,
    pub frame: Frame,
    /// Text the OCR has to find, lowercase.
    pub expected: Vec<String>,
}

struct Page {
    lines: Vec<Line>,
    expected: &'static [&'static str],
    dark: bool,
}

struct Line {
    x: f64,
    y: f64,
    size: f64,
    mono: bool,
    gray: f64,
    text: String,
}

fn line(x: f64, y: f64, size: f64, text: impl Into<String>) -> Line {
    Line {
        x,
        y,
        size,
        mono: false,
        gray: 0.1,
        text: text.into(),
    }
}

const EN_WORDS: &[&str] = &[
    "please", "find", "the", "attached", "report", "for", "this", "quarter", "and", "let", "me",
    "know", "if", "anything", "looks", "wrong", "before", "we", "send", "it", "to", "the",
    "customer", "on", "Friday", "morning",
];
const DE_WORDS: &[&str] = &[
    "bitte", "senden", "sie", "uns", "den", "Bericht", "für", "dieses", "Quartal", "und", "melden",
    "sie", "sich", "falls", "etwas", "nicht", "stimmt", "bevor", "wir", "ihn", "dem", "Kunden",
    "am", "Freitag", "zusenden",
];

/// A sentence of `count` words that differs from row to row.
fn filler(words: &[&str], row: usize, count: usize) -> String {
    (0..count)
        .map(|i| words[(row * 7 + i * 3) % words.len()])
        .collect::<Vec<_>>()
        .join(" ")
}

fn page(kind: Kind, language: Language) -> Page {
    let german = language == Language::German;
    let words = if german { DE_WORDS } else { EN_WORDS };
    let mut lines = Vec::new();
    match kind {
        Kind::Email => {
            lines.push(line(
                40.0,
                30.0,
                20.0,
                if german {
                    "Betreff: Rechnung 4711 und Zahlungserinnerung"
                } else {
                    "Subject: Invoice 4711 and payment reminder"
                },
            ));
            lines.push(line(
                40.0,
                62.0,
                14.0,
                if german {
                    "Von: Anna Schmidt <anna.schmidt@beispiel.de>"
                } else {
                    "From: Mark Taylor <mark.taylor@example.com>"
                },
            ));
            for row in 0..32 {
                lines.push(line(
                    40.0,
                    110.0 + row as f64 * 24.0,
                    15.0,
                    filler(words, row, 12),
                ));
            }
        }
        Kind::Spreadsheet => {
            let head = if german {
                ["Quartal", "Umsatz", "Kosten", "Marge", "Kunde"]
            } else {
                ["Quarter", "Revenue", "Costs", "Margin", "Customer"]
            };
            for (col, title) in head.iter().enumerate() {
                lines.push(line(40.0 + col as f64 * 260.0, 30.0, 14.0, *title));
            }
            for row in 0..34 {
                let values = [
                    format!("Q{} 2024", row % 4 + 1),
                    format!("{}.{:03},00", 100 + row, (row * 37) % 1000),
                    format!("{}.{:03},50", 60 + row, (row * 91) % 1000),
                    format!("{}.{:03},10", 20 + row, (row * 53) % 1000),
                    format!("Kunde {:04}", 1000 + row * 3),
                ];
                for (col, value) in values.iter().enumerate() {
                    lines.push(line(
                        40.0 + col as f64 * 260.0,
                        62.0 + row as f64 * 24.0,
                        14.0,
                        value,
                    ));
                }
            }
        }
        Kind::Invoice => {
            lines.push(line(
                60.0,
                50.0,
                30.0,
                if german {
                    "RECHNUNG 2024-0815"
                } else {
                    "INVOICE 2024-0815"
                },
            ));
            lines.push(line(
                60.0,
                100.0,
                14.0,
                "Muster GmbH, Hauptstraße 12, 10115 Berlin",
            ));
            for row in 0..16 {
                let name = if german {
                    "Beratung Stunde"
                } else {
                    "Consulting hour"
                };
                lines.push(line(
                    60.0,
                    180.0 + row as f64 * 30.0,
                    15.0,
                    format!("{name} {}", row + 1),
                ));
                lines.push(line(
                    900.0,
                    180.0 + row as f64 * 30.0,
                    15.0,
                    format!("{},00 EUR", 90 + row),
                ));
            }
            lines.push(line(
                60.0,
                720.0,
                20.0,
                if german {
                    "Gesamtbetrag 1.190,00 EUR"
                } else {
                    "Total 1,190.00 EUR"
                },
            ));
            lines.push(line(60.0, 770.0, 14.0, "IBAN DE89 3704 0044 0532 0130 00"));
        }
        Kind::Editor => {
            let code = [
                "fn total(items: &[Item]) -> Result<Money, Error> {",
                "    let mut sum = Money::zero();",
                "    for item in items {",
                "        sum = sum.checked_add(item.price)?;",
                "    }",
                "    Ok(sum)",
                "}",
            ];
            for row in 0..36 {
                let mut text = line(
                    70.0,
                    30.0 + row as f64 * 22.0,
                    14.0,
                    code[row % code.len()].to_string(),
                );
                text.mono = true;
                text.gray = 0.85;
                lines.push(text);
                let mut number = line(30.0, 30.0 + row as f64 * 22.0, 14.0, format!("{}", row + 1));
                number.mono = true;
                number.gray = 0.5;
                lines.push(number);
            }
        }
    }
    let expected: &'static [&'static str] = match (kind, german) {
        (Kind::Email, false) => &["invoice 4711", "payment reminder", "mark.taylor"],
        (Kind::Email, true) => &["rechnung 4711", "zahlungserinnerung", "anna.schmidt"],
        (Kind::Spreadsheet, false) => &["quarter", "revenue", "costs"],
        (Kind::Spreadsheet, true) => &["quartal", "umsatz", "kosten"],
        (Kind::Invoice, false) => &["invoice 2024-0815", "total 1,190.00 eur", "de89 3704"],
        (Kind::Invoice, true) => &["rechnung 2024-0815", "gesamtbetrag", "de89 3704"],
        (Kind::Editor, _) => &["fn total", "let mut sum"],
    };
    Page {
        lines,
        expected,
        dark: kind == Kind::Editor,
    }
}

/// One window at `width` x `height` pixels. The layout is drawn for 1440x900 points, so the
/// larger size is the same window on a Retina display.
pub fn render(kind: Kind, language: Language, width: usize, height: usize) -> Result<Fixture> {
    let page = page(kind, language);
    let scale = width as f64 / 1440.0;
    let space = CGColorSpace::new_device_rgb().context("no color space")?;
    // SAFETY: with no data pointer Core Graphics allocates and owns the pixels.
    let context = unsafe {
        CGBitmapContextCreate(
            std::ptr::null_mut(),
            width,
            height,
            8,
            0,
            Some(&space),
            CGImageAlphaInfo::PremultipliedLast.0,
        )
    }
    .context("no bitmap context")?;
    let background = if page.dark { 0.12 } else { 1.0 };
    CGContext::set_rgb_fill_color(Some(&context), background, background, background, 1.0);
    CGContext::fill_rect(
        Some(&context),
        CGRect::new(
            CGPoint::new(0.0, 0.0),
            CGSize::new(width as f64, height as f64),
        ),
    );
    CGContext::translate_ctm(Some(&context), 0.0, height as f64);
    CGContext::scale_ctm(Some(&context), scale, -scale);

    autoreleasepool(|_| {
        let graphics = NSGraphicsContext::graphicsContextWithCGContext_flipped(&context, true);
        NSGraphicsContext::saveGraphicsState_class();
        NSGraphicsContext::setCurrentContext(Some(&graphics));
        for line in &page.lines {
            draw(line);
        }
        NSGraphicsContext::restoreGraphicsState_class();
    });

    let image: CFRetained<_> = CGBitmapContextCreateImage(Some(&context)).context("no image")?;
    Ok(Fixture {
        name: format!("{kind:?} {language:?} {width}x{height}"),
        frame: Frame::new(image),
        expected: page.expected.iter().map(|text| text.to_string()).collect(),
    })
}

fn draw(line: &Line) {
    let font = if line.mono {
        // SAFETY: reads a constant exported by AppKit.
        NSFont::monospacedSystemFontOfSize_weight(line.size, unsafe { NSFontWeightRegular })
    } else {
        NSFont::systemFontOfSize(line.size)
    };
    let color = NSColor::colorWithSRGBRed_green_blue_alpha(line.gray, line.gray, line.gray, 1.0);
    // SAFETY: the keys are AppKit's attribute names and the values have the types they expect.
    unsafe {
        let attributes = NSDictionary::from_slices(
            &[NSFontAttributeName, NSForegroundColorAttributeName],
            &[
                AsRef::<AnyObject>::as_ref(&*font),
                AsRef::<AnyObject>::as_ref(&*color),
            ],
        );
        NSString::from_str(&line.text)
            .drawAtPoint_withAttributes(CGPoint::new(line.x, line.y), Some(&attributes));
    }
}
