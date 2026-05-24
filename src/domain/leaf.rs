// Domain entity I/O for leaf documents.
//
// A leaf is a collected document produced by `bo add`. It lives at
// {output_dir}/{slug}.md and has YAML frontmatter followed by a markdown body.
//
// Analogous to branch.rs; together they define the two entity types in bo's
// knowledge graph.
//
// The title field is always double-quoted in the written YAML so that special
// characters (colons, embedded quotes) are escaped consistently. This is the
// canonical on-disk format for leaf files and is preserved by patch_fields
// when bo compile updates the frontmatter later.

use crate::domain::{Timestamp, Title, Url};

/// Format leaf document content — frontmatter block followed by body.
///
/// Kept private; callers should use `write`. Separated only so that the
/// formatting logic can be exercised in tests without touching the filesystem.
pub fn format_content(
    title: Option<&Title>,
    url: &Url,
    collected_at: &Timestamp,
    body: &str,
    summary: Option<&str>,
) -> String {
    let title_yaml = match title {
        Some(t) => {
            let escaped = t
                .as_str()
                .replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace('\n', "\\n")
                .replace('\t', "\\t")
                .replace('\r', "\\r");
            format!("\"{}\"", escaped)
        }
        None => "\"\"".to_string(),
    };

    let ts_str = collected_at.to_rfc3339_millis();

    let mut doc = String::new();
    doc.push_str("---\n");
    doc.push_str(&format!("title: {}\n", title_yaml));
    doc.push_str(&format!("url: {}\n", url.as_str()));
    doc.push_str(&format!("collected_at: {}\n", ts_str));
    doc.push_str(&format!("updated_at: {}\n", ts_str));

    if let Some(s) = summary {
        if !s.is_empty() {
            if s.contains('\n') {
                doc.push_str("summary: |\n");
                for line in s.lines() {
                    doc.push_str("  ");
                    doc.push_str(line);
                    doc.push('\n');
                }
            } else {
                let escaped = s.replace('\\', "\\\\").replace('"', "\\\"");
                doc.push_str(&format!("summary: \"{}\"\n", escaped));
            }
        }
    }

    doc.push_str("---\n\n");

    if let Some(t) = title {
        doc.push_str(&format!("# {}\n\n", t.as_str()));
    }

    doc.push_str(body);
    if !body.ends_with('\n') {
        doc.push('\n');
    }

    doc
}

#[cfg(test)]
#[path = "../tests/domain_leaf_tests.rs"]
mod tests;
