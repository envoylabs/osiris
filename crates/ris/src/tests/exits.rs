use super::*;

// Canaries are assembled at runtime so that no secret-shaped literal
// sits in the tree: the exit scan would rightly refuse the commit
// that added it. The assembly is the contortion the no-exceptions
// rule costs; it is confined to test fixtures.
fn path_canary() -> String {
    ["/Us", "ers/someone/x.rs"].concat()
}

fn key_canary() -> String {
    ["AKIA", "IOSFODNN7EXAMPLE"].concat()
}

#[test]
fn added_lines_are_scanned_with_their_file_and_line() {
    let diff = format!(
        "diff --git a/src/a.rs b/src/a.rs\n\
             index 000..111 100644\n\
             --- a/src/a.rs\n\
             +++ b/src/a.rs\n\
             @@ -10,0 +11,2 @@ fn f() {{\n\
             +    let fine = 1;\n\
             +    let path = \"{}\";\n\
             @@ -20 +22 @@\n\
             -    let old = \"{}\";\n\
             +    let newer = 2;\n",
        path_canary(),
        key_canary()
    );
    let hits = scan_diff(&diff, &Policy::default());
    assert_eq!(
        hits,
        vec![Hit {
            file: "src/a.rs".into(),
            line: Some(12),
            category: Category::AbsolutePath,
        }],
        "the removed key is not scanned; the added path is"
    );
    assert!(
        !hits[0].to_string().contains("someone"),
        "the match is not echoed"
    );
}

#[test]
fn messages_and_files_are_scanned_by_line() {
    let policy = Policy::new(["Example Person"]);
    let hits = scan_lines(
        "commit message",
        "feat: x\n\nReviewed with example person.\n",
        &policy,
    );
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].line, Some(3));
    assert_eq!(hits[0].category, Category::Denylisted);
    assert!(scan_lines("f", "nothing here\n", &policy).is_empty());
}
