use pkms_core::*;
use std::thread::sleep;
use std::time::Duration;
use tempfile::tempdir;
use uuid::Uuid;

fn id() -> String {
    Uuid::new_v4().to_string()
}

fn profile() -> GenerationProfile {
    GenerationProfile {
        channel_id: None,
        capabilities: None,
        provider: "openai_chat".into(),
        endpoint: "https://example.test/v1".into(),
        model: "synthetic".into(),
        model_digest: String::new(),
        config_revision: 1,
        temperature: 0.0,
        context_tokens: 4096,
        chat_token_limit: None,
    }
}

/// One finished answer, so a rating can be given for it.
fn answered(db: &mut Library, question: &str) -> String {
    let accepted = db
        .accept_answer(
            &id(),
            StartAnswer {
                max_output_tokens: None,
                citation_count: None,
                search: SearchQuery {
                    query: question.into(),
                    mode: SearchMode::Lexical,
                    scope: SearchScope::default(),
                    limit: 8,
                    minimum_cosine: None,
                },
                model: Some(profile()),
                agent: Some(AgentOptions::default()),
            },
        )
        .unwrap();
    db.claim_answer(&accepted.answer.id).unwrap().unwrap();
    // A rating is accepted once the answer is no longer running, so the shortest
    // terminal answer is enough here; the report does not care which path made it
    // terminal.
    db.cancel_answer(&id(), &accepted.answer.id).unwrap();
    accepted.answer.id
}

fn rate(db: &mut Library, answer_id: &str, rating: i32, comment: &str) {
    db.feedback_answer(
        &id(),
        AnswerFeedback {
            answer_id: answer_id.into(),
            rating,
            comment: comment.into(),
            updated_at: None,
        },
    )
    .unwrap();
    // The page orders by the receipt time, so a rating has to be separable from
    // the one before it instead of relying on the answer id tiebreak.
    sleep(Duration::from_millis(5));
}

#[test]
fn collected_ratings_are_paged_newest_change_first_with_the_question() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let first = answered(&mut db, "第一个问题");
    let second = answered(&mut db, "第二个问题");
    let third = answered(&mut db, "第三个问题");
    rate(&mut db, &first, 1, "很有用");
    rate(&mut db, &second, -1, "");
    rate(&mut db, &third, -1, "答非所问");

    let page = db.feedback_page(None, 2).unwrap();
    assert_eq!(
        page.items
            .iter()
            .map(|row| row.answer_id.as_str())
            .collect::<Vec<_>>(),
        vec![third.as_str(), second.as_str()],
        "newest rating change first"
    );
    assert_eq!(page.items[0].question, "第三个问题");
    assert_eq!(page.items[0].rating, -1);
    assert_eq!(page.items[0].comment, "答非所问");
    assert!(
        page.items[0].changed_at.len() > 10,
        "a receipt time, not a placeholder"
    );
    let cursor = page.next_cursor.expect("a full page has a cursor");

    let rest = db.feedback_page(Some(&cursor), 2).unwrap();
    assert_eq!(
        rest.items
            .iter()
            .map(|row| row.answer_id.as_str())
            .collect::<Vec<_>>(),
        vec![first.as_str()]
    );
    assert_eq!(rest.items[0].question, "第一个问题");
    assert_eq!(rest.next_cursor, None, "the last page reports no cursor");

    let summary = db.feedback_summary().unwrap();
    assert_eq!(summary.total, 3);
    assert_eq!(summary.helpful, 1);
    assert_eq!(summary.unhelpful, 2);
    assert_eq!(summary.neutral, 0);
    assert_eq!(summary.commented, 2);
    assert_eq!(summary.newest_at, Some(page.items[0].changed_at.clone()));
}

#[test]
fn changing_a_rating_upserts_the_row_and_moves_it_to_the_front() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let answer = answered(&mut db, "同一个问题");
    rate(&mut db, &answer, 1, "先给了好评");
    let before = db.feedback_page(None, 10).unwrap();
    assert_eq!(db.feedback_summary().unwrap().helpful, 1);

    rate(&mut db, &answer, -1, "后来发现不对");
    let after = db.feedback_page(None, 10).unwrap();
    assert_eq!(after.items.len(), 1, "the rating is updated, not appended");
    assert_eq!(after.items[0].rating, -1);
    assert_eq!(after.items[0].comment, "后来发现不对");
    assert!(
        after.items[0].changed_at > before.items[0].changed_at,
        "the change time comes from the newest receipt"
    );
    let summary = db.feedback_summary().unwrap();
    assert_eq!(summary.total, 1);
    assert_eq!(summary.helpful, 0);
    assert_eq!(summary.unhelpful, 1);
    assert_eq!(summary.commented, 1);
}

#[test]
fn an_empty_report_is_all_zero_and_a_bad_cursor_is_rejected() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let page = db.feedback_page(None, 30).unwrap();
    assert!(page.items.is_empty());
    assert_eq!(page.next_cursor, None);
    let summary = db.feedback_summary().unwrap();
    assert_eq!(summary.total, 0);
    assert_eq!(summary.newest_at, None);

    assert!(db.feedback_page(Some("not-a-cursor"), 30).is_err());
    assert!(
        db.feedback_page(Some("2026-01-01T00:00:00Z|not-a-uuid"), 30)
            .is_err()
    );
    assert!(db.feedback_page(None, 0).is_err());
    assert!(db.feedback_page(None, 101).is_err());

    let answer = answered(&mut db, "问题");
    rate(&mut db, &answer, 1, "");
    let page = db.feedback_page(None, 1).unwrap();
    assert_eq!(
        page.next_cursor, None,
        "a page that ends the list has no cursor"
    );
}
