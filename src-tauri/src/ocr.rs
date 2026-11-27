//! Text recognition (OCR) on captures, with the OCR built into the OS: the Vision
//! framework on macOS and Windows.Media.Ocr on Windows; on Linux, Tesseract when it
//! is installed. Nothing is bundled and nothing leaves the computer.

use image::RgbaImage;
use serde::Serialize;

/// A recognized word, in pixels of the image that was read.
#[derive(Debug, Clone, Serialize)]
pub struct Word {
    pub text: String,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// A line of text, as a list of words in reading order.
#[derive(Debug, Clone, Serialize)]
pub struct Line {
    pub words: Vec<Word>,
}

/// Reads the text in `img`. Lines come in the OS's reading order.
pub fn recognize(img: &RgbaImage) -> Result<Vec<Line>, String> {
    if img.width() < 8 || img.height() < 8 {
        return Ok(Vec::new());
    }
    platform::recognize(img)
}

/// Splits `text` into words, with each word's range in UTF-16 units (what Cocoa uses)
/// and in characters.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn split_words(text: &str) -> Vec<(String, std::ops::Range<usize>, std::ops::Range<usize>)> {
    let mut out = Vec::new();
    let (mut u16_pos, mut char_pos) = (0usize, 0usize);
    let mut current: Option<(String, usize, usize)> = None;
    for c in text.chars() {
        if c.is_whitespace() {
            if let Some((word, u, ch)) = current.take() {
                out.push((word, u..u16_pos, ch..char_pos));
            }
        } else {
            current
                .get_or_insert_with(|| (String::new(), u16_pos, char_pos))
                .0
                .push(c);
        }
        u16_pos += c.len_utf16();
        char_pos += 1;
    }
    if let Some((word, u, ch)) = current {
        out.push((word, u..u16_pos, ch..char_pos));
    }
    out
}

#[cfg(target_os = "macos")]
mod platform {
    use image::codecs::png::CompressionType;
    use objc2::{rc::autoreleasepool, AnyThread};
    use objc2_foundation::{NSArray, NSData, NSDictionary, NSRange, NSString};
    use objc2_vision::{
        VNImageRequestHandler, VNRecognizeTextRequest, VNRequest, VNRequestTextRecognitionLevel,
    };

    use super::{split_words, Line, Word};

    /// Smallest text Vision should look for, in pixels. Vision's default (1/32 of
    /// the image height) skips normal UI text on a full-screen capture.
    const MIN_TEXT_PX: f32 = 8.0;

    pub fn recognize(img: &image::RgbaImage) -> Result<Vec<Line>, String> {
        let (iw, ih) = (img.width() as f64, img.height() as f64);
        let png = crate::capture::encode_png(img, CompressionType::Fast)?;
        autoreleasepool(|_| {
            let data = NSData::with_bytes(&png);
            let handler = VNImageRequestHandler::initWithData_options(
                VNImageRequestHandler::alloc(),
                &data,
                &NSDictionary::new(),
            );
            let request = VNRecognizeTextRequest::new();
            request.setRecognitionLevel(VNRequestTextRecognitionLevel::Accurate);
            request.setUsesLanguageCorrection(true);
            request.setRecognitionLanguages(&NSArray::from_retained_slice(&[
                NSString::from_str("pt-BR"),
                NSString::from_str("en-US"),
            ]));
            request.setMinimumTextHeight((MIN_TEXT_PX / ih as f32).min(1.0 / 32.0));
            let as_request: &VNRequest = &request;
            handler
                .performRequests_error(&NSArray::from_slice(&[as_request]))
                .map_err(|e| {
                    format!(
                        "falha no reconhecimento de texto: {}",
                        e.localizedDescription()
                    )
                })?;

            let mut lines = Vec::new();
            let Some(results) = request.results() else {
                return Ok(lines);
            };
            for observation in results.iter() {
                let Some(candidate) = observation.topCandidates(1).firstObject() else {
                    continue;
                };
                let text = candidate.string().to_string();
                // Normalized, origin at the bottom left: flip to pixels from the top left.
                let to_px = |r: objc2_core_foundation::CGRect| {
                    (
                        (r.origin.x * iw) as f32,
                        ((1.0 - r.origin.y - r.size.height) * ih) as f32,
                        (r.size.width * iw) as f32,
                        (r.size.height * ih) as f32,
                    )
                };
                let (lx, ly, lw, lh) = to_px(unsafe { observation.boundingBox() });
                let total_chars = text.chars().count().max(1) as f32;
                let words = split_words(&text)
                    .into_iter()
                    .map(|(word, u16_range, chars)| {
                        let range = NSRange::new(u16_range.start, u16_range.len());
                        let (x, y, w, h) =
                            match unsafe { candidate.boundingBoxForRange_error(range) } {
                                Ok(rect) => to_px(unsafe { rect.boundingBox() }),
                                // Fall back to a share of the line, by character count.
                                Err(_) => (
                                    lx + lw * chars.start as f32 / total_chars,
                                    ly,
                                    lw * chars.len() as f32 / total_chars,
                                    lh,
                                ),
                            };
                        Word {
                            text: word,
                            x,
                            y,
                            w,
                            h,
                        }
                    })
                    .collect::<Vec<_>>();
                if !words.is_empty() {
                    lines.push(Line { words });
                }
            }
            Ok(lines)
        })
    }
}

#[cfg(target_os = "windows")]
mod platform {
    use std::sync::Once;

    use image::{imageops, RgbaImage};
    use windows::{
        core::HSTRING,
        Globalization::Language,
        Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap},
        Media::Ocr::OcrEngine,
        Storage::Streams::DataWriter,
        Win32::System::Com::CoIncrementMTAUsage,
    };

    use super::{Line, Word};

    fn engine() -> Result<OcrEngine, String> {
        // The user's display languages first (Portuguese for most Kadro users, which
        // reads English text too), then English or Portuguese if installed, then
        // whatever OCR language Windows has.
        if let Ok(engine) = OcrEngine::TryCreateFromUserProfileLanguages() {
            return Ok(engine);
        }
        for tag in ["pt-BR", "en-US"] {
            if let Ok(lang) = Language::CreateLanguage(&HSTRING::from(tag)) {
                if let Ok(engine) = OcrEngine::TryCreateFromLanguage(&lang) {
                    return Ok(engine);
                }
            }
        }
        if let Ok(langs) = OcrEngine::AvailableRecognizerLanguages() {
            for lang in langs {
                if let Ok(engine) = OcrEngine::TryCreateFromLanguage(&lang) {
                    return Ok(engine);
                }
            }
        }
        Err(
            "O Windows não tem nenhum idioma de reconhecimento de texto instalado. \
             Adicione o português ou o inglês em Configurações > Hora e idioma > Idioma."
                .into(),
        )
    }

    pub fn recognize(img: &RgbaImage) -> Result<Vec<Line>, String> {
        // WinRT needs the thread in an apartment; keep a process-wide MTA alive.
        static MTA: Once = Once::new();
        MTA.call_once(|| unsafe {
            let _ = CoIncrementMTAUsage();
        });
        let err = |e: windows::core::Error| format!("falha no reconhecimento de texto: {e}");

        let engine = engine()?;
        // Windows OCR misses small text: screen text at 100% scaling is ~12 px
        // tall, so read the image at twice the size when it fits.
        let max = OcrEngine::MaxImageDimension().unwrap_or(2600).max(64);
        let longest = img.width().max(img.height());
        let scale = if longest * 2 <= max {
            2.0
        } else if longest > max {
            max as f32 / longest as f32
        } else {
            1.0
        };
        let scaled;
        let src = if scale == 1.0 {
            img
        } else {
            let w = ((img.width() as f32 * scale).round() as u32).max(1);
            let h = ((img.height() as f32 * scale).round() as u32).max(1);
            scaled = imageops::resize(img, w, h, imageops::FilterType::Triangle);
            &scaled
        };

        // RGBA to BGRA, which is what SoftwareBitmap and the OCR engine take.
        let mut bgra = src.as_raw().clone();
        for px in bgra.chunks_exact_mut(4) {
            px.swap(0, 2);
        }
        let writer = DataWriter::new().map_err(err)?;
        writer.WriteBytes(&bgra).map_err(err)?;
        let buffer = writer.DetachBuffer().map_err(err)?;
        let bitmap = SoftwareBitmap::CreateCopyFromBuffer(
            &buffer,
            BitmapPixelFormat::Bgra8,
            src.width() as i32,
            src.height() as i32,
        )
        .map_err(err)?;
        let result = engine
            .RecognizeAsync(&bitmap)
            .map_err(err)?
            .join()
            .map_err(err)?;

        let mut lines = Vec::new();
        for line in result.Lines().map_err(err)? {
            let mut words = Vec::new();
            for word in line.Words().map_err(err)? {
                let r = word.BoundingRect().map_err(err)?;
                words.push(Word {
                    text: word.Text().map_err(err)?.to_string(),
                    x: r.X / scale,
                    y: r.Y / scale,
                    w: r.Width / scale,
                    h: r.Height / scale,
                });
            }
            if !words.is_empty() {
                lines.push(Line { words });
            }
        }
        Ok(lines)
    }
}

/// Linux has no OCR of its own: Kadro uses Tesseract when it is installed (most
/// distributions package it), so nothing heavy ships with the app.
#[cfg(target_os = "linux")]
mod platform {
    use std::{
        io::Write,
        process::{Command, Stdio},
        sync::OnceLock,
    };

    use image::{imageops, RgbaImage};

    use super::{Line, Word};

    const MISSING: &str = "Para copiar texto de imagens no Linux, instale o Tesseract:\n\n\
        Ubuntu/Debian: sudo apt install tesseract-ocr tesseract-ocr-por\n\
        Fedora: sudo dnf install tesseract tesseract-langpack-por\n\
        Arch: sudo pacman -S tesseract tesseract-data-por tesseract-data-eng";

    /// The languages to read: Portuguese and English when installed.
    fn languages() -> Option<&'static str> {
        static LANGS: OnceLock<Option<String>> = OnceLock::new();
        LANGS
            .get_or_init(|| {
                let out = Command::new("tesseract").arg("--list-langs").output().ok()?;
                let text = String::from_utf8_lossy(&out.stdout).to_string()
                    + &String::from_utf8_lossy(&out.stderr);
                let installed: Vec<&str> = text.lines().map(str::trim).collect();
                let wanted: Vec<&str> = ["por", "eng"]
                    .into_iter()
                    .filter(|l| installed.contains(l))
                    .collect();
                Some(wanted.join("+"))
            })
            .as_deref()
    }

    pub fn recognize(img: &RgbaImage) -> Result<Vec<Line>, String> {
        let langs = languages().ok_or_else(|| MISSING.to_string())?;
        // Screen text is small for Tesseract, which was made for scans: reading it
        // at two or three times the size is much more accurate.
        let side = img.width().max(img.height());
        let scale = match side {
            0..=1400 => 3,
            1401..=2800 => 2,
            _ => 1,
        };
        let input = if scale > 1 {
            imageops::resize(
                img,
                img.width() * scale,
                img.height() * scale,
                imageops::FilterType::CatmullRom,
            )
        } else {
            img.clone()
        };
        let png = crate::capture::encode_png(&input, image::codecs::png::CompressionType::Fast)?;

        let mut cmd = Command::new("tesseract");
        // Sparse text: screenshots are UI and scattered labels more than pages. Lines are
        // put together afterwards from the word positions.
        cmd.args(["stdin", "stdout", "--psm", "11"]);
        if !langs.is_empty() {
            cmd.args(["-l", langs]);
        }
        let mut child = cmd
            .arg("tsv")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| MISSING.to_string())?;
        let mut stdin = child.stdin.take().ok_or("tesseract")?;
        let writer = std::thread::spawn(move || stdin.write_all(&png));
        let out = child.wait_with_output().map_err(|e| e.to_string())?;
        let _ = writer.join();
        if !out.status.success() {
            return Err("O Tesseract não conseguiu ler a imagem.".into());
        }
        Ok(parse_tsv(&String::from_utf8_lossy(&out.stdout), scale as f32))
    }

    /// Reads Tesseract's TSV (one row per word, level 5) into lines. Tesseract's own
    /// grouping follows its layout guesses (a column of dates becomes a "block"), so
    /// words are grouped by position instead: same height on screen, same line, read
    /// left to right. Boxes are divided by `scale`.
    pub fn parse_tsv(tsv: &str, scale: f32) -> Vec<Line> {
        let mut words: Vec<Word> = Vec::new();
        for row in tsv.lines().skip(1) {
            let cols: Vec<&str> = row.splitn(12, '\t').collect();
            if cols.len() < 12 || cols[0] != "5" {
                continue;
            }
            let text = cols[11].trim();
            if text.is_empty() {
                continue;
            }
            let num = |i: usize| cols[i].parse::<f32>().unwrap_or(0.0);
            words.push(Word {
                text: text.to_string(),
                x: num(6) / scale,
                y: num(7) / scale,
                w: num(8) / scale,
                h: num(9) / scale,
            });
        }
        words.sort_by(|a, b| (a.y + a.h / 2.0).total_cmp(&(b.y + b.h / 2.0)));
        // (top, bottom) of each line, and its words.
        let mut lines: Vec<(f32, f32, Vec<Word>)> = Vec::new();
        for word in words {
            let (top, bottom) = (word.y, word.y + word.h);
            let fits = lines.iter_mut().rev().take(3).find(|(t, b, _)| {
                let overlap = bottom.min(*b) - top.max(*t);
                overlap > 0.5 * word.h.min(*b - *t)
            });
            match fits {
                Some((t, b, list)) => {
                    *t = t.min(top);
                    *b = b.max(bottom);
                    list.push(word);
                }
                None => lines.push((top, bottom, vec![word])),
            }
        }
        lines.sort_by(|a, b| a.0.total_cmp(&b.0));
        lines
            .into_iter()
            .map(|(_, _, mut words)| {
                words.sort_by(|a, b| a.x.total_cmp(&b.x));
                Line { words }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::split_words;

    #[cfg(target_os = "linux")]
    #[test]
    fn reads_tesseract_tsv() {
        let tsv = "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n\
            5\t1\t2\t1\t1\t1\t90\t12\t80\t20\t96\tmundo\n\
            5\t1\t1\t1\t1\t1\t20\t10\t60\t20\t95\tOlá\n\
            4\t1\t1\t1\t2\t0\t20\t40\t60\t20\t-1\t\n\
            5\t1\t1\t1\t2\t1\t20\t40\t60\t20\t90\tde novo\n";
        let lines = super::platform::parse_tsv(tsv, 2.0);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].words.len(), 2);
        assert_eq!(lines[0].words[1].text, "mundo");
        assert_eq!(lines[0].words[1].x, 45.0);
        assert_eq!(lines[1].words[0].text, "de novo");
    }

    #[test]
    fn splits_words_with_utf16_ranges() {
        let words = split_words("Olá,  mundo 😀x");
        let texts: Vec<_> = words.iter().map(|w| w.0.as_str()).collect();
        assert_eq!(texts, ["Olá,", "mundo", "😀x"]);
        assert_eq!(words[1].1, 6..11);
        // The emoji takes two UTF-16 units but one character.
        assert_eq!(words[2].1, 12..15);
        assert_eq!(words[2].2, 12..14);
    }
}
