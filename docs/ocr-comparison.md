# OCR comparison (O1)

Written on 2026-11-02. Decision: keep Apple Vision `.fast`. No candidate beats it on the fixtures and the budget, so O1 does not switch engines.

## What Sextant needs

- Screen text of the focused window, English and German, as lines with a box and a confidence (plan.md §5).
- p95 under 250 ms per window, off the main thread (plan.md §11), on a MacBook at 1440x900 and 2880x1800.
- No download for the default path, no network at run time, no Python.
- Text a person reads on screen: UI labels, mail, spreadsheets, invoices, code. Not scanned documents or handwriting.

## Apple Vision today, measured

`cargo run -p sextant-app --release -- bench`, synthetic fixtures (email, spreadsheet, invoice and editor, each in English and German), 4 timed passes each:

| Size      | Level    | p50         | p95          | Expected text not found |
| --------- | -------- | ----------- | ------------ | ----------------------- |
| 1440x900  | fast     | 47 to 63 ms | 71 to 101 ms | 0                       |
| 1440x900  | accurate | 447 ms      | 914 ms       | 0                       |
| 2880x1800 | fast     | 63 to 75 ms | 87 to 95 ms  | 0                       |
| 2880x1800 | accurate | 465 ms      | 989 ms       | 0                       |

One run right after a release build put 2880x1800 `.fast` at 308 ms p95. Two runs on an idle machine gave 87 and 95 ms. The budget holds with room to spare, but a busy machine eats that room.

`.fast` finds every expected string on every fixture. The fixtures cannot show an engine that reads better, because there is nothing left to find.

## Candidates

Not measured here: this environment cannot reach crates.io or the model hosts. The notes come from each project's own documentation.

| Engine                                                                       | How it would run                                       | Against Vision                                                                                                                                                                                                                                                                                                             |
| ---------------------------------------------------------------------------- | ------------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Apple Vision `.accurate`                                                     | Already wired, one constant                            | Same text on the fixtures, 9 to 10 times slower, over the budget.                                                                                                                                                                                                                                                          |
| PaddleOCR PP-OCRv5 (detection plus recognition, ONNX)                        | ONNX Runtime, which Sextant already bundles for speech | A widely used open option for screen text. Needs a model download (breaks "no download for the default path"), detection post-processing and CTC decoding in Rust, and runs on the CPU or Core ML, not Vision's tuned Neural Engine path. Published speeds are for servers and GPUs, not a laptop under the 250 ms budget. |
| `ocrs` (Rust, rten runtime)                                                  | A crate plus a model download                          | Latin script only, built for clean documents; the project describes itself as early and slower than mature engines.                                                                                                                                                                                                        |
| Tesseract 5                                                                  | A C++ library through bindings                         | Weaker on anti-aliased screen text and small UI fonts; needs a language pack per language.                                                                                                                                                                                                                                 |
| OCR vision-language models (PaddleOCR-VL, GOT-OCR 2.0, olmOCR, DeepSeek-OCR) | ONNX or MLX, hundreds of MB to several GB              | Read layout and tables well, but take seconds per page on a laptop. Far over 250 ms per frame.                                                                                                                                                                                                                             |
| Surya, docTR, EasyOCR                                                        | Python and PyTorch                                     | Out: no Python in the app.                                                                                                                                                                                                                                                                                                 |

## Why no switch

- Vision already meets the budget at about a third of it and reads every fixture string.
- Every open engine adds a download, code to maintain, and CPU or ANE time that Vision gets from the OS.
- The plan's reason for Vision (plan.md §2: ships with the OS, no download, Neural Engine) still holds.

## What would reopen O1

1. Harder fixtures that `.fast` fails: small 11 px UI text, dark mode, dense spreadsheets, code with symbols, mixed English and German lines, low-contrast labels. Add them to `app/src/platform/render.rs` and record the misses.
2. If `.fast` misses there, try `.accurate` on only the frames that miss (low confidence or few lines) before any new engine.
3. Only then measure PP-OCRv5 through the bundled ONNX Runtime on the same fixtures, with the model behind the same download consent as the speech models.
