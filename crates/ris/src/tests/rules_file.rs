use std::path::Path;

use super::*;

#[test]
fn the_rendered_file_is_the_template_then_the_projects_part() {
    let out = render("docs/own.md", "AGENTS.md", "\n# Here\n\nOur own rule.\n\n");
    let mut lines = out.lines();
    let first = lines.next().unwrap();
    assert!(first.starts_with("<!-- Rendered by `ris rules --render docs/own.md --to AGENTS.md`"));
    assert!(first.ends_with("-->"), "{first}");
    assert_eq!(lines.next(), Some(""));
    // The template whole, then the part, one blank line between, and
    // one newline at the end.
    let body = out.split_once("-->\n\n").unwrap().1;
    assert_eq!(
        body,
        format!("{}\n\n# Here\n\nOur own rule.\n", RULES.trim_end())
    );
}

#[test]
fn rendering_twice_gives_the_same_file() {
    let once = render("a.md", "b.md", "x");
    assert_eq!(once, render("a.md", "b.md", "x"));
    assert_ne!(once, render("a.md", "b.md", "y"));
}

#[test]
fn only_a_path_inside_the_directory_is_named() {
    assert_eq!(named(Path::new("docs/own.md")).unwrap(), "docs/own.md");
    assert_eq!(named(Path::new("./AGENTS.md")).unwrap(), "AGENTS.md");
    // Built from pieces: a rooted path never stands in the tree.
    let rooted = format!("{}tmp{}own.md", '/', '/');
    for bad in [rooted.as_str(), "../own.md", "docs/../../own.md"] {
        let err = named(Path::new(bad)).unwrap_err().to_string();
        assert!(err.starts_with("refused:"), "{bad}: {err}");
    }
}

#[test]
fn a_rendered_file_names_its_source_and_itself() {
    let out = render("docs/own.md", "AGENTS.md", "x");
    assert_eq!(
        sources(&out),
        Some(("docs/own.md".to_owned(), "AGENTS.md".to_owned()))
    );
    // A file written by hand names nothing, wherever the words stand.
    assert_eq!(sources("# Rules\n"), None);
    assert_eq!(sources(&format!("# Rules\n{out}")), None);
    assert_eq!(sources(""), None);
}

#[test]
fn only_rendered_rules_files_are_rendered_again() {
    let dir = std::env::temp_dir()
        .canonicalize()
        .unwrap()
        .join(format!("ris-rules-again-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("docs")).unwrap();
    let put = |file: &str, text: &str| std::fs::write(dir.join(file), text).unwrap();
    let get = |file: &str| std::fs::read_to_string(dir.join(file)).unwrap();
    put("docs/own.md", "The first rule.\n");
    put("AGENTS.md", &render("docs/own.md", "AGENTS.md", "x"));
    // Written by hand; and one that names a file other than itself,
    // as a copy of a rendered file would.
    put("CLAUDE.md", "@AGENTS.md\n");
    let copy = render("docs/own.md", "elsewhere.md", "x");
    put("OSIRIS.md", &copy);

    // The project's part is where it was rendered from: nothing to do.
    put(
        "AGENTS.md",
        &render("docs/own.md", "AGENTS.md", &get("docs/own.md")),
    );
    assert_eq!(rerender(&dir).unwrap(), vec![]);

    put("docs/own.md", "The second rule.\n");
    assert_eq!(
        stale(&dir, Path::new("docs/own.md"), Path::new("AGENTS.md"))
            .unwrap()
            .unwrap(),
        format!(
            "AGENTS.md is not what the rules and docs/own.md render; they first differ at line {}:\n  \
             in the file: \"The first rule.\"\n  rendered:    \"The second rule.\"",
            get("AGENTS.md").lines().count()
        )
    );
    assert_eq!(
        rerender(&dir).unwrap(),
        vec![("AGENTS.md".to_owned(), "docs/own.md".to_owned())]
    );
    assert_eq!(
        get("AGENTS.md"),
        render("docs/own.md", "AGENTS.md", "The second rule.\n")
    );
    assert_eq!(
        stale(&dir, Path::new("docs/own.md"), Path::new("AGENTS.md")).unwrap(),
        None
    );
    assert_eq!(get("CLAUDE.md"), "@AGENTS.md\n");
    assert_eq!(get("OSIRIS.md"), copy);
    assert!(!dir.join("elsewhere.md").exists());

    // A rendered file whose source is gone is an error, not a file
    // left stale in silence.
    std::fs::remove_file(dir.join("docs/own.md")).unwrap();
    let err = rerender(&dir).unwrap_err().to_string();
    assert!(err.contains("docs/own.md"), "{err}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn write_renders_into_the_file_and_says_whether_it_changed() {
    let dir = std::env::temp_dir()
        .canonicalize()
        .unwrap()
        .join(format!("ris-rules-file-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("docs")).unwrap();
    let (source, to) = (Path::new("docs/own.md"), Path::new("RULES.md"));

    // No project part yet: nothing is written.
    let err = write(&dir, source, to).unwrap_err().to_string();
    assert!(err.contains("docs/own.md"), "{err}");
    assert!(!dir.join(to).exists());

    std::fs::write(dir.join(source), "Our own rule.\n").unwrap();
    assert!(write(&dir, source, to).unwrap());
    let written = std::fs::read_to_string(dir.join(to)).unwrap();
    assert_eq!(
        written,
        render("docs/own.md", "RULES.md", "Our own rule.\n")
    );
    assert!(
        !write(&dir, source, to).unwrap(),
        "unchanged the second time"
    );

    // A hand edit is rendered away.
    std::fs::write(dir.join(to), format!("{written}edited\n")).unwrap();
    assert!(write(&dir, source, to).unwrap());
    assert_eq!(std::fs::read_to_string(dir.join(to)).unwrap(), written);
    let _ = std::fs::remove_dir_all(&dir);
}
