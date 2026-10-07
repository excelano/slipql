// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use slipql::slpc::toml_edit::DocumentMut;
use slipql::{evaluate, parse_condition, Truth};

fn flyleaf() -> DocumentMut {
    r#"
slipcase_version = "1.1"

[content]
file = "invoice.pdf"

[records]
created = 2024-01-17
tags = ["legal", "vendor"]

[records.custodian]
email = "jdoe@example.com"
"#
    .parse()
    .unwrap()
}

fn truth(condition: &str) -> Truth {
    evaluate(
        &parse_condition(condition).unwrap(),
        &flyleaf(),
        "contracts/invoice.pdf.slpc",
    )
    .0
}

#[test]
fn a_condition_on_its_own_is_evaluated_against_one_document() {
    assert_eq!(
        truth(
            r#"records.custodian.email in ("jdoe@example.com", "x@example.com") and records.created >= 2023-01-01"#
        ),
        Truth::True
    );
    assert_eq!(truth("records.created >= 2025-01-01"), Truth::False);
    assert_eq!(truth(r#"records.tags contains "legal""#), Truth::True);
    assert_eq!(truth("exists records.marking"), Truth::False);
    assert_eq!(truth(r#"records.marking = "Confidential""#), Truth::Unknown);
    assert_eq!(truth(r#"@path like "contracts/%""#), Truth::True);
    assert_eq!(truth(r#"not (records.created < 2020-01-01)"#), Truth::True);
}

#[test]
fn a_comparison_across_types_decides_nothing_and_is_tallied() {
    let (t, tally) = evaluate(
        &parse_condition(r#"records.created = "2024-01-17""#).unwrap(),
        &flyleaf(),
        "",
    );
    assert_eq!(t, Truth::Unknown);
    assert!(!tally.is_empty());
}

#[test]
fn a_query_is_not_a_condition() {
    assert!(parse_condition("select @path from '.'").is_err());
    assert!(parse_condition("records.created >= 2023-01-01 limit 3").is_err());
    assert!(parse_condition("").is_err());
}
