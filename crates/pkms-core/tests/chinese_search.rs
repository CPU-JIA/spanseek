use pkms_core::*;
use std::collections::BTreeSet;
use tempfile::tempdir;

fn ids(hits: &[SearchHit]) -> BTreeSet<String> {
    hits.iter().map(|hit| hit.note.id.clone()).collect()
}

/// The tokenizer shipped before this change: jieba only, no folding, no pinyin.
fn legacy_tokens(jieba: &jieba_rs::Jieba, text: &str) -> BTreeSet<String> {
    jieba
        .cut_for_search(text, true)
        .into_iter()
        .map(|word| word.word.to_lowercase())
        .collect()
}

#[test]
fn simplified_traditional_and_pinyin_queries_reach_the_same_notes() {
    let root = tempdir().unwrap();
    let mut library = Library::open(root.path()).unwrap();
    let simplified = library
        .create_note("知识库设计", "知识库是本地优先的资料库，支持检索。")
        .unwrap();
    let traditional = library
        .create_note("知識庫筆記", "知識庫與資料檢索的繁體寫法。")
        .unwrap();
    let rag = library
        .create_note("检索增强生成", "检索增强生成结合外部证据。")
        .unwrap();
    library
        .create_note("Local search", "local first knowledge base")
        .unwrap();

    let folded = library.search("知识库", 10).unwrap();
    let folded_ids = ids(&folded);
    assert!(
        folded_ids.contains(&simplified.id) && folded_ids.contains(&traditional.id),
        "simplified query must reach both scripts: {folded_ids:?}"
    );

    let pinyin = library.search("zhishiku", 10).unwrap();
    let pinyin_ids = ids(&pinyin);
    assert!(
        pinyin_ids.contains(&simplified.id) && pinyin_ids.contains(&traditional.id),
        "pinyin query must reach both scripts: {pinyin_ids:?}"
    );

    let term = library.search("检索增强生成", 10).unwrap();
    assert!(
        ids(&term).contains(&rag.id),
        "compound term must stay searchable"
    );

    // Before/after: the old tokenizer produced no overlap for these pairs.
    let jieba = jieba_rs::Jieba::new();
    assert!(
        legacy_tokens(&jieba, "知识库")
            .is_disjoint(&legacy_tokens(&jieba, "知識庫與資料檢索的繁體寫法。")),
        "legacy tokenizer cannot match traditional text with a simplified query"
    );
    assert!(
        legacy_tokens(&jieba, "zhishiku").is_disjoint(&legacy_tokens(
            &jieba,
            "知识库是本地优先的资料库，支持检索。"
        )),
        "legacy tokenizer has no pinyin tokens"
    );
}
