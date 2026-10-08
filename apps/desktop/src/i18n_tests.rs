//! Garante que toda string `@tr` da interface tem tradução pt-BR (S-013).

use std::collections::HashSet;
use std::path::Path;

/// Lê um literal de string Slint a partir de `s` (logo após a aspa inicial).
fn read_literal(s: &str) -> Option<String> {
    let mut out = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => out.push(chars.next()?),
            '"' => return Some(out),
            c => out.push(c),
        }
    }
    None
}

fn tr_ids(source: &str) -> Vec<String> {
    source
        .match_indices("@tr(\"")
        .filter_map(|(i, m)| read_literal(&source[i + m.len()..]))
        .collect()
}

fn po_ids(po: &str) -> HashSet<String> {
    po.lines()
        .filter_map(|l| l.strip_prefix("msgid \""))
        .filter_map(read_literal)
        .filter(|id| !id.is_empty())
        .collect()
}

#[test]
fn every_ui_string_has_pt_br_translation() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let po =
        std::fs::read_to_string(root.join("translations/pt-BR/LC_MESSAGES/vellcast-desktop.po"))
            .expect("catálogo pt-BR");
    let translated = po_ids(&po);
    let mut missing = Vec::new();
    for entry in std::fs::read_dir(root.join("ui")).expect("ui/") {
        let path = entry.expect("entrada").path();
        if path.extension().is_some_and(|e| e == "slint") {
            let source = std::fs::read_to_string(&path).expect("slint");
            for id in tr_ids(&source) {
                if !translated.contains(&id) {
                    missing.push(format!("{}: {id}", path.display()));
                }
            }
        }
    }
    assert!(
        missing.is_empty(),
        "strings sem tradução pt-BR:\n{}",
        missing.join("\n")
    );
}

#[test]
fn literal_reader_handles_escapes() {
    assert_eq!(
        read_literal(r#"a \"b\" c" rest"#).as_deref(),
        Some(r#"a "b" c"#)
    );
    assert_eq!(
        tr_ids(r#"x: @tr("Hi {}", n); y: @tr("Bye");"#),
        vec!["Hi {}", "Bye"]
    );
}
