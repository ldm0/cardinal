mod common;
use cardinal_syntax::*;
use common::*;

#[test]
fn recognized_filter_kinds_without_arguments() {
    let cases = [
        ("file:", FilterKind::File),
        ("folder:", FilterKind::Folder),
        ("audio:", FilterKind::Audio),
        ("video:", FilterKind::Video),
        ("doc:", FilterKind::Doc),
        ("exe:", FilterKind::Exe),
        ("attribdupe:", FilterKind::AttributeDuplicate),
        ("dmdupe:", FilterKind::DateModifiedDuplicate),
        ("dupe:", FilterKind::Duplicate),
        ("namepartdupe:", FilterKind::NamePartDuplicate),
        ("sizedupe:", FilterKind::SizeDuplicate),
        ("nowholefilename:", FilterKind::NoWholeFilename),
    ];

    for (q, kind) in cases {
        let expr = parse_ok(q);
        filter_is_kind(&expr, &kind);
        filter_arg_none(&expr);
    }
}

#[test]
fn custom_filter_name_is_preserved() {
    let expr = parse_ok("proj:");
    filter_is_custom(&expr, "proj");
}

#[test]
fn ext_list_is_semicolon_split() {
    let expr = parse_ok("ext:jpg;png;jpeg");
    filter_is_kind(&expr, &FilterKind::Ext);
    filter_arg_is_list(&expr, &["jpg", "png", "jpeg"]);
}

#[test]
fn content_filter_has_bare_argument() {
    let expr = parse_ok("content:error");
    filter_is_kind(&expr, &FilterKind::Content);
    filter_arg_raw(&expr, "error");
}

#[test]
fn phrase_argument_is_detected() {
    let expr = parse_ok("parent:\"/Users/demo\"");
    let (_, arg) = filter_kind(&expr);
    match arg.as_ref().unwrap().kind {
        ArgumentKind::Phrase => {}
        ref other => panic!("expected Phrase, got: {other:?}"),
    }
}

#[test]
fn filter_can_appear_anywhere_in_and_chain() {
    let expr = parse_ok("video: size:>1gb report");
    let parts = as_and(&expr);
    filter_is_kind(&parts[0], &FilterKind::Video);
    filter_arg_none(&parts[0]);
    filter_is_kind(&parts[1], &FilterKind::Size);
    filter_arg_is_comparison(&parts[1], ComparisonOp::Gt, "1gb");
    word_is(&parts[2], "report");
}
