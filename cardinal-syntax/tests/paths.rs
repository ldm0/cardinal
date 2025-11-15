mod common;
use common::*;

#[test]
fn parses_unix_paths_as_words() {
    let expr = parse_ok("/Users/demo/Documents report");
    let parts = as_and(&expr);
    assert_eq!(parts.len(), 2);
    word_is(&parts[0], "/Users/demo/Documents");
    word_is(&parts[1], "report");
}

#[test]
fn parses_multiple_unix_paths_in_or() {
    let expr = parse_ok("/Volumes/Data OR /Users");
    let parts = as_or(&expr);
    assert_eq!(parts.len(), 2);
    word_is(&parts[0], "/Volumes/Data");
    word_is(&parts[1], "/Users");
}

#[test]
fn parses_unc_paths_and_child_filter() {
    let expr = parse_ok("\\\\srv\\share child:*.mp3");
    let parts = as_and(&expr);
    word_is(&parts[0], "\\\\srv\\share");
    filter_is_kind(&parts[1], &cardinal_syntax::FilterKind::Child);
    filter_arg_raw(&parts[1], "*.mp3");
}

#[test]
fn parses_windows_drive_roots_and_dirs() {
    let expr = parse_ok("D:");
    filter_is_custom(&expr, "D");
    filter_arg_none(&expr);

    let expr = parse_ok(r"D:\\Music\\");
    filter_is_custom(&expr, "D");
    filter_arg_raw(&expr, r"\\Music\\");
}

#[test]
fn parses_env_expanded_style_segments_as_words() {
    let expr = parse_ok("%TEMP%\\*.log");
    match &expr {
        cardinal_syntax::Expr::Term(cardinal_syntax::Term::Word(w)) => {
            assert_eq!(w, "%TEMP%\\*.log")
        }
        other => panic!("unexpected {other:?}"),
    }
}
