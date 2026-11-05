use pkms_sync::{
    CURRENT_AUTHORITY_SCHEMA, MIN_SUPPORTED_AUTHORITY_SCHEMA, SYNC_PROTOCOL_VERSION,
    SchemaDecision, classify_authority_database,
};
use serde_json::json;

fn decision_label(decision: SchemaDecision) -> String {
    match decision {
        SchemaDecision::Create => "create".into(),
        SchemaDecision::Migrate { from } => format!("migrate_from_{from}"),
        SchemaDecision::Current => "current".into(),
        SchemaDecision::Reject(reason) => format!("reject_{}", reason.code()),
    }
}

fn main() {
    let matrix: Vec<_> = (0..=8)
        .map(|version| {
            json!({
                "version": version,
                "decision": decision_label(classify_authority_database(
                    1_347_112_258,
                    version,
                    false,
                )),
            })
        })
        .collect();
    let empty = decision_label(classify_authority_database(0, 0, true));
    let future = decision_label(classify_authority_database(
        1_347_112_258,
        CURRENT_AUTHORITY_SCHEMA + 1,
        false,
    ));
    let foreign = decision_label(classify_authority_database(
        99,
        CURRENT_AUTHORITY_SCHEMA,
        false,
    ));
    let all_checks_passed = MIN_SUPPORTED_AUTHORITY_SCHEMA == 1
        && CURRENT_AUTHORITY_SCHEMA == 7
        && SYNC_PROTOCOL_VERSION == 1
        && empty == "create"
        && future == "reject_future_schema"
        && foreign == "reject_foreign_application";
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "format": "v192-compatibility-verification-v1",
            "sync_protocol_version": SYNC_PROTOCOL_VERSION,
            "minimum_authority_schema": MIN_SUPPORTED_AUTHORITY_SCHEMA,
            "current_authority_schema": CURRENT_AUTHORITY_SCHEMA,
            "empty_database": empty,
            "future_database": future,
            "foreign_database": foreign,
            "matrix": matrix,
            "all_checks_passed": all_checks_passed,
        }))
        .unwrap()
    );
}
