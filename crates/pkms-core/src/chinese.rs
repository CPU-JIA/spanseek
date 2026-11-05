//! Chinese-specific search helpers: traditional-to-simplified folding, pinyin
//! tokens for ASCII queries and an optional workspace term dictionary.
//!
//! The token stream stays in `note_fts`/`source_fts`, so these helpers change
//! what gets indexed and queried without adding a schema column. Workspaces
//! indexed by an older build need an index rebuild to pick them up.

use pinyin::ToPinyin;
use std::{
    fs,
    path::{Path, PathBuf},
};

const MAX_TERM_CHARS: usize = 64;
const MAX_TERMS: usize = 5_000;
const MAX_PINYIN_RUN: usize = 32;

/// Folds traditional Chinese to simplified so `知識庫` and `知识库` share the
/// same tokens regardless of which form the author or the query uses.
pub fn fold(text: &str) -> String {
    fast2s::convert(text)
}

/// Emits a toneless pinyin token for one Han word, so a query typed as
/// `zhishiku` matches an indexed `知识库`. Mixed-script words are skipped to
/// avoid polluting the token stream.
pub fn pinyin_token(word: &str) -> Option<String> {
    if word.chars().count() > MAX_PINYIN_RUN {
        return None;
    }
    let mut syllables = String::new();
    for ch in word.chars() {
        let pinyin = ch.to_pinyin()?;
        syllables.push_str(pinyin.plain());
    }
    if syllables.is_empty() || syllables.len() > 128 {
        return None;
    }
    Some(syllables)
}
/// Reads a user term dictionary (one term per line, `#` starts a comment).
pub fn dictionary_words(path: &Path) -> Vec<String> {
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter(|line| line.chars().count() <= MAX_TERM_CHARS)
        .take(MAX_TERMS)
        .map(str::to_owned)
        .collect()
}

/// Optional term file: `PKMS_SEARCH_TERMS` wins, otherwise the per-user data
/// directory is used. Missing files are not an error.
pub fn dictionary_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("PKMS_SEARCH_TERMS") {
        return Some(PathBuf::from(path));
    }
    let base = std::env::var_os("LOCALAPPDATA").or_else(|| std::env::var_os("XDG_DATA_HOME"))?;
    Some(PathBuf::from(base).join("PKMS").join("search-terms.txt"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use jieba_rs::Jieba;

    #[test]
    fn folds_traditional_to_simplified() {
        assert_eq!(fold("知識庫與資料"), "知识库与资料");
        assert_eq!(fold("已经简体"), "已经简体");
    }

    #[test]
    fn pinyin_tokens_cover_han_words_only() {
        assert_eq!(pinyin_token("知识库").as_deref(), Some("zhishiku"));
        assert_eq!(pinyin_token("hello"), None);
        assert_eq!(pinyin_token("知识库a"), None);
    }

    #[test]
    fn dictionary_words_skip_comments_and_blanks() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("terms.txt");
        fs::write(&path, "# 术语\n检索增强生成\n\n  ").unwrap();
        assert_eq!(dictionary_words(&path), ["检索增强生成"]);
        assert!(dictionary_words(&dir.path().join("missing.txt")).is_empty());
    }

    #[test]
    fn a_dictionary_term_changes_segmentation() {
        let mut jieba = Jieba::new();
        let before = jieba
            .cut_for_search("检索增强生成", true)
            .into_iter()
            .map(|word| word.word)
            .collect::<Vec<_>>();
        assert!(!before.contains(&"检索增强生成"), "{before:?}");
        jieba.add_word("检索增强生成", None, None);
        let after = jieba
            .cut_for_search("检索增强生成", true)
            .into_iter()
            .map(|word| word.word)
            .collect::<Vec<_>>();
        assert!(after.contains(&"检索增强生成"), "{after:?}");
    }
}
