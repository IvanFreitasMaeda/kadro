//! Text recognition (OCR) on captures, with the OCR built into the OS: the Vision
//! framework on macOS and Windows.Media.Ocr on Windows. Nothing is bundled and
//! nothing leaves the computer.

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

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod platform {
    use super::Line;

    pub fn recognize(_img: &image::RgbaImage) -> Result<Vec<Line>, String> {
        Err("O reconhecimento de texto ainda não está disponível neste sistema.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::split_words;

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
