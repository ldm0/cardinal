mod common;
use cardinal_syntax::*;
use common::*;

#[test]
fn dotted_range_in_size() {
    let expr = parse_ok("size:1mb..10mb");
    filter_is_kind(&expr, &FilterKind::Size);
    filter_arg_is_range_dots(&expr, Some("1mb"), Some("10mb"));
}

#[test]
fn dotted_open_ended_ranges() {
    let cases = [
        ("size:..10mb", None, Some("10mb")),
        ("size:1mb..", Some("1mb"), None),
    ];
    for (q, s, e) in cases {
        let expr = parse_ok(q);
        filter_is_kind(&expr, &FilterKind::Size);
        filter_arg_is_range_dots(&expr, s, e);
    }
}

#[test]
fn hyphenated_ranges_for_dates() {
    let expr = parse_ok("dc:2014/8/1-2014/8/31");
    filter_is_kind(&expr, &FilterKind::DateCreated);
    filter_arg_is_range_hyphen(&expr, "2014/8/1", "2014/8/31");

    let expr = parse_ok("dm:2023-01-01..2023-12-31");
    filter_is_kind(&expr, &FilterKind::DateModified);
    filter_arg_is_range_dots(&expr, Some("2023-01-01"), Some("2023-12-31"));
}

#[test]
fn comparisons_are_detected() {
    let expr = parse_ok("size:>1GB");
    filter_is_kind(&expr, &FilterKind::Size);
    filter_arg_is_comparison(&expr, ComparisonOp::Gt, "1GB");

    let expr = parse_ok("width:<=4000");
    filter_is_kind(&expr, &FilterKind::Width);
    filter_arg_is_comparison(&expr, ComparisonOp::Lte, "4000");

    let expr = parse_ok("size:=10mb");
    filter_is_kind(&expr, &FilterKind::Size);
    filter_arg_is_comparison(&expr, ComparisonOp::Eq, "10mb");

    let expr = parse_ok("size:!=10mb");
    filter_is_kind(&expr, &FilterKind::Size);
    filter_arg_is_comparison(&expr, ComparisonOp::Ne, "10mb");
}

#[test]
fn lone_operator_is_treated_as_bare_argument() {
    let expr = parse_ok("size:>");
    filter_is_kind(&expr, &FilterKind::Size);
    filter_arg_raw(&expr, ">");
}
