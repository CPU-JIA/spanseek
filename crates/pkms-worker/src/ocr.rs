//! Bounded local OCR through a pinned Tesseract executable.
//!
//! Tesseract is invoked only for pages whose PDF text layer is empty. The app
//! supplies the executable and tessdata directory; a document cannot choose a
//! binary, language pack or working directory.
use crate::{OcrConfig, Result, error};
use std::{
    io::Read,
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

const MAX_TSV_BYTES: u64 = 16 * 1024 * 1024;
const MAX_STDERR_BYTES: u64 = 64 * 1024;
const PAGE_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct OcrWord {
    pub text: String,
    pub left: f64,
    pub top: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct OcrLine {
    pub text: String,
    pub words: Vec<OcrWord>,
}

/// Parses Tesseract TSV word rows into reading-order lines.
pub(crate) fn tsv_lines(tsv: &str) -> Vec<OcrLine> {
    let mut lines: Vec<(u32, u32, u32, OcrLine)> = Vec::new();
    for row in tsv.lines().skip(1) {
        let fields: Vec<&str> = row.split('\t').collect();
        if fields.len() < 12 || fields[0].parse::<u32>().ok() != Some(5) {
            continue;
        }
        let Ok(block) = fields[2].parse::<u32>() else {
            continue;
        };
        let Ok(paragraph) = fields[3].parse::<u32>() else {
            continue;
        };
        let Ok(line) = fields[4].parse::<u32>() else {
            continue;
        };
        let Ok(left) = fields[6].parse::<f64>() else {
            continue;
        };
        let Ok(top) = fields[7].parse::<f64>() else {
            continue;
        };
        let Ok(width) = fields[8].parse::<f64>() else {
            continue;
        };
        let Ok(height) = fields[9].parse::<f64>() else {
            continue;
        };
        let confidence = fields[10].parse::<f64>().unwrap_or(-1.0);
        let text = fields[11..].join("\t");
        let text = text.trim();
        if confidence < 0.0 || text.is_empty() || width <= 0.0 || height <= 0.0 {
            continue;
        }
        let word = OcrWord {
            text: text.to_owned(),
            left,
            top,
            width,
            height,
        };
        match lines.last_mut() {
            Some((b, p, l, existing)) if (*b, *p, *l) == (block, paragraph, line) => {
                append_word(existing, word);
            }
            _ => lines.push((
                block,
                paragraph,
                line,
                OcrLine {
                    text: word.text.clone(),
                    words: vec![word],
                },
            )),
        }
    }
    lines.into_iter().map(|(_, _, _, line)| line).collect()
}

fn append_word(line: &mut OcrLine, word: OcrWord) {
    if space_before_append(&line.words, &word) {
        line.text.push(' ');
    }
    line.text.push_str(&word.text);
    line.words.push(word);
}

/// Whether the word at `index` needs a separating space from its predecessor.
pub(crate) fn space_before(words: &[OcrWord], word: &OcrWord, index: usize) -> bool {
    let Some(previous) = index.checked_sub(1).and_then(|index| words.get(index)) else {
        return false;
    };
    let gap = word.left - (previous.left + previous.width);
    if !needs_space(Some(previous.text.as_str()), &word.text) {
        return false;
    }
    let previous_char = previous.text.chars().last();
    let next_char = word.text.chars().next();
    // A CJK/Latin boundary is a real word boundary even when the scanner
    // reports adjacent boxes; within the same script require a visible gap.
    previous_char.is_some_and(is_cjk) != next_char.is_some_and(is_cjk)
        || gap > word.height.max(previous.height) * 0.35
}

fn space_before_append(words: &[OcrWord], word: &OcrWord) -> bool {
    space_before(words, word, words.len())
}

fn needs_space(previous: Option<&str>, next: &str) -> bool {
    let Some(previous) = previous.and_then(|text| text.chars().last()) else {
        return false;
    };
    let Some(next) = next.chars().next() else {
        return false;
    };
    !(is_cjk(previous) && is_cjk(next)
        || is_cjk(previous) && is_cjk_punctuation(next)
        || is_cjk_punctuation(previous) && is_cjk(next))
}

fn is_cjk(c: char) -> bool {
    matches!(c as u32, 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF)
}

fn is_cjk_punctuation(c: char) -> bool {
    matches!(
        c,
        '，' | '。'
            | '、'
            | '；'
            | '：'
            | '？'
            | '！'
            | '“'
            | '”'
            | '‘'
            | '’'
            | '（'
            | '）'
            | '《'
            | '》'
    )
}

pub(crate) fn recognize(config: &OcrConfig, image: &Path) -> Result<Vec<OcrLine>> {
    let mut child = Command::new(&config.executable)
        .arg(image)
        .arg("stdout")
        .arg("-l")
        .arg(&config.languages)
        .arg("--tessdata-dir")
        .arg(&config.tessdata_dir)
        .arg("--psm")
        .arg("3")
        .arg("-c")
        .arg("preserve_interword_spaces=1")
        .arg("-c")
        .arg("tessedit_create_tsv=1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| error("ocr_unavailable"))?;
    let stdout = child.stdout.take().ok_or_else(|| error("ocr_failed"))?;
    let stderr = child.stderr.take().ok_or_else(|| error("ocr_failed"))?;
    let stdout_reader = std::thread::spawn(move || crate::read_bounded(stdout, MAX_TSV_BYTES));
    let stderr_reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stderr.take(MAX_STDERR_BYTES).read_to_end(&mut bytes);
        bytes
    });
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|_| error("ocr_failed"))? {
            break status;
        }
        if started.elapsed() >= PAGE_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error("ocr_timeout"));
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let bytes = stdout_reader
        .join()
        .map_err(|_| error("ocr_failed"))?
        .map_err(|_| error("ocr_output_limit"))?;
    let _ = stderr_reader.join();
    if !status.success() {
        return Err(error("ocr_failed"));
    }
    let text = String::from_utf8(bytes).map_err(|_| error("ocr_invalid_output"))?;
    Ok(tsv_lines(&text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tsv_keeps_reading_order_and_builds_natural_chinese_spacing() {
        let tsv = "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n\
5\t1\t1\t1\t1\t1\t10\t10\t80\t20\t96.4\t保留\n\
5\t1\t1\t1\t1\t2\t92\t10\t80\t20\t95.1\t证据\n\
5\t1\t1\t1\t1\t3\t176\t10\t60\t20\t94.0\t2026\n\
5\t1\t1\t1\t2\t1\t10\t40\t100\t20\t93.0\tSecond line\n";
        let lines = tsv_lines(tsv);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].text, "保留证据 2026");
        assert_eq!(lines[1].text, "Second line");
        assert_eq!(lines[0].words.len(), 3);
    }

    #[test]
    fn malformed_rows_and_empty_text_are_ignored() {
        let tsv = "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n\
5\t1\t1\t1\t1\t1\t10\t10\t0\t20\t96\tempty-width\n\
5\t1\t1\t1\t1\t1\t10\t10\t20\t20\t-1\tno-confidence\n\
not-a-row\n";
        assert!(tsv_lines(tsv).is_empty());
    }
}
