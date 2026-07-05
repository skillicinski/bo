use crate::domain::branch;
use crate::domain::{Timestamp, Title};

// ── goldens: byte-stable on-disk format ──────────────────────────────────

#[test]
fn golden_branch() {
    let content = branch::format_content(
        &Title::parse("Example Branch").unwrap(),
        "# Example Branch\n\nBranch body.",
        &["leaf-a.md".to_string(), "leaf-b.md".to_string()],
        &Timestamp::parse("2025-01-01T00:00:00.000Z").unwrap(),
        &Timestamp::parse("2025-06-01T12:00:00.000Z").unwrap(),
    );
    assert_eq!(
        content,
        "---\ntitle: Example Branch\ncreated_at: 2025-01-01T00:00:00.000Z\nupdated_at: 2025-06-01T12:00:00.000Z\nleaves:\n- leaf-a.md\n- leaf-b.md\n---\n\n# Example Branch\n\nBranch body."
    );
}
