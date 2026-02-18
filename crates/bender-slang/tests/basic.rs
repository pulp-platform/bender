use std::path::PathBuf;

fn fixture_path(rel: &str) -> String {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tests/pickle")
        .join(rel)
        .canonicalize()
        .expect("valid fixture path")
        .to_string_lossy()
        .into_owned()
}

#[test]
fn parse_valid_file_succeeds() {
    let mut session = bender_slang::SlangSession::new();
    let files = vec![fixture_path("src/top.sv")];
    let includes = vec![fixture_path("include")];
    let defines = vec![];
    assert!(session.parse_group(&files, &includes, &defines).is_ok());
    assert_eq!(session.tree_count(), 1);
}

#[test]
fn parse_invalid_file_returns_parse_error() {
    let mut session = bender_slang::SlangSession::new();
    let files = vec![fixture_path("src/broken.sv")];
    let includes = vec![];
    let defines = vec![];
    let result = session.parse_group(&files, &includes, &defines);

    match result {
        Err(bender_slang::SlangError::ParseGroup { .. }) => {}
        Err(other) => panic!("expected SlangError::ParseGroup, got {other}"),
        Ok(_) => panic!("expected parse to fail"),
    }
}
