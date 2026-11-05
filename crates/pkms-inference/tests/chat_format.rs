use pkms_inference::protocol::ChatFormat;

#[test]
fn claims_schema_matches_the_independently_probed_contract() {
    let format = ChatFormat::EvidenceAnswer {
        citation_ids: vec!["E1".into()],
    };
    let expected: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/claims-v1-schema.json")).unwrap();
    assert_eq!(format.json_schema().unwrap(), expected);
    let all = (1..=32).map(|id| format!("E{id}")).collect::<Vec<_>>();
    let schema = ChatFormat::EvidenceAnswer {
        citation_ids: all.clone(),
    }
    .json_schema()
    .unwrap();
    for branch in schema["anyOf"].as_array().unwrap() {
        let ids = &branch["properties"]["claims"]["items"]["properties"]["citation_ids"];
        assert_eq!(ids["items"]["enum"], serde_json::json!(all));
        assert_eq!(ids["maxItems"], 32);
    }
}

#[test]
fn format_rejects_invalid_or_ambiguous_evidence_allowlists() {
    for ids in [vec![], vec!["E1", "E1"], vec!["E1"; 33]] {
        let format = ChatFormat::EvidenceAnswer {
            citation_ids: ids.into_iter().map(str::to_owned).collect(),
        };
        assert!(format.json_schema().is_err());
    }
    for id in [
        "",
        "E0",
        "E33",
        "E01",
        "E+1",
        "E-1",
        "e1",
        "E１",
        "E1\0",
        "E1\n",
        "E9999999999999999999999",
    ] {
        let format = ChatFormat::EvidenceAnswer {
            citation_ids: vec![id.into()],
        };
        assert!(format.json_schema().is_err(), "accepted {id:?}");
    }
    for value in [
        serde_json::json!({"kind":"json_schema","schema":{}}),
        serde_json::json!({"kind":"evidence_answer","citation_ids":["E1"],"schema":{}}),
        serde_json::json!({"kind":"evidence_answer"}),
    ] {
        assert!(serde_json::from_value::<ChatFormat>(value).is_err());
    }
}
