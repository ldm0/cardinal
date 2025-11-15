mod common;
use cardinal_syntax::*;
use common::*;

#[test]
fn keyword_boundaries_do_not_consume_neighbors() {
    let cases = [
        "fooANDbar",
        "foo ANDbar",
        "fooAND bar",
        "foo NOTbar",
        "NOTfoo",
        "barORbaz",
        "bar ORbaz",
        "barOR baz",
    ];

    fn assert_no_keyword_nodes(expr: &Expr) {
        match expr {
            Expr::Not(_) => panic!("unexpected NOT node from keyword boundary"),
            Expr::Or(_) => panic!("unexpected OR node from keyword boundary"),
            Expr::And(parts) => parts.iter().for_each(assert_no_keyword_nodes),
            Expr::Term(_) | Expr::Empty => {}
        }
    }

    for q in cases {
        let expr = parse_ok(q);
        assert_no_keyword_nodes(&expr);
    }
}

#[test]
fn multiple_spaces_and_tabs_are_treated_as_and() {
    let expr = parse_ok("foo\t\tbar   baz\nqux");
    let parts = as_and(&expr);
    assert_eq!(parts.len(), 4);
    word_is(&parts[0], "foo");
    word_is(&parts[1], "bar");
    word_is(&parts[2], "baz");
    word_is(&parts[3], "qux");
}

#[test]
fn not_prefix_chains_of_various_lengths() {
    for n in 0..6 {
        let q = format!("{}x", "!".repeat(n));
        let expr = parse_ok(&q);
        if n % 2 == 0 {
            word_is(&expr, "x");
        } else {
            let inner = as_not(&expr);
            word_is(inner, "x");
        }
    }
}

#[test]
fn or_sequences_with_varied_spacing() {
    let cases = [
        ("a|b|c", 3),
        ("a |b | c", 3),
        (" a| b |c ", 3),
        ("|a|b|", 4),
        ("||", 3),
    ];

    for (q, count) in cases {
        let expr = parse_ok(q);
        let parts = as_or(&expr);
        assert_eq!(parts.len(), count);
    }
}

#[test]
fn filters_interleave_with_terms_and_groups() {
    let expr = parse_ok("(foo bar) size:>1mb <D:|E:> ext:jpg;png baz");
    let parts = as_and(&expr);
    // (foo bar)
    let p0 = as_and(&parts[0]);
    assert_eq!(p0.len(), 2);
    // size:>1mb
    filter_is_kind(&parts[1], &FilterKind::Size);
    filter_arg_is_comparison(&parts[1], ComparisonOp::Gt, "1mb");
    // <D:|E:>
    let or_parts = as_or(&parts[2]);
    assert_eq!(or_parts.len(), 2);
    filter_is_custom(&or_parts[0], "D");
    filter_is_custom(&or_parts[1], "E");
    // ext:jpg;png
    filter_is_kind(&parts[3], &FilterKind::Ext);
    filter_arg_is_list(&parts[3], &["jpg", "png"]);
    // baz
    word_is(&parts[4], "baz");
}

#[test]
fn many_dimension_values_and_ranges() {
    let cases = [
        "dimensions:800x600",
        "dimensions:1024x768",
        "dimensions:1920x1080",
        "dimensions:3840x2160",
        "width:>100",
        "width:>=100",
        "width:=100",
        "width:!=100",
        "height:<100",
        "height:<=100",
        "size:1..10",
        "size:..10",
        "size:1..",
    ];
    for q in cases {
        let _ = parse_ok(q);
    }
}

#[test]
fn date_filters_allow_hyphen_ranges_only_for_dates() {
    let ok_cases = [
        "dc:2020/1/1-2020/12/31",
        "dm:2021/01/01-2021/02/01",
        "da:2022-01-01-2022-12-31",
        "dr:2023-03-01-2023-03-15",
    ];
    for q in ok_cases {
        let _ = parse_ok(q);
    }

    // size: should not accept hyphen as range unless date-like (stays bare)
    let expr = parse_ok("size:10-20");
    let (_, arg) = common::filter_kind(&expr);
    assert!(matches!(
        arg.as_ref().unwrap().kind,
        ArgumentKind::Bare | ArgumentKind::Comparison(_)
    ));
}
