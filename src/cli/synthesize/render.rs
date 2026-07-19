// ── synthesis output rendering ───────────────────────────────────────────────

use std::io::{self, Write};

use super::types::NO_NEW_LEAVES_REASON;
use super::{SynthesisPreview, SynthesisResult};

/// Render stderr-bound diagnostic/progress lines (title-collision warnings,
/// transaction-recovery notices, per-branch write progress) collected during the
/// run. The pipeline never prints; the caller renders these post-run.
pub fn render_diagnostics<W: Write>(lines: &[String], stderr: &mut W) -> io::Result<()> {
    for line in lines {
        writeln!(stderr, "{}", line)?;
    }
    Ok(())
}

fn write_notifications<W: Write>(notifications: &[String], stdout: &mut W) -> io::Result<()> {
    for note in notifications {
        writeln!(stdout, "\u{2192} {}", note)?;
    }
    Ok(())
}

pub fn render_human<W: Write>(
    result: &SynthesisResult,
    stdout: &mut W,
    tree_name: &str,
) -> io::Result<()> {
    if result.status == "noop" {
        match result.reason.as_deref() {
            Some("empty_tree") => writeln!(stdout, "{} is empty", tree_name)?,
            Some("single_leaf") => writeln!(stdout, "{} only has 1 leaf", tree_name)?,
            Some(NO_NEW_LEAVES_REASON) => writeln!(stdout, "nothing new to synthesize")?,
            _ => writeln!(stdout, "synthesized: no work to do")?,
        };
        write_notifications(&result.notifications, stdout)?;
        return Ok(());
    }

    render_summary_human(result, stdout)?;
    write_notifications(&result.notifications, stdout)?;
    Ok(())
}

fn render_summary_human<W: Write>(result: &SynthesisResult, stdout: &mut W) -> io::Result<()> {
    if let (Some(mode), Some(model)) = (&result.mode, &result.model) {
        writeln!(stdout, "synthesized ({mode:?}) using {model}")?;
    } else {
        writeln!(stdout, "synthesized")?;
    }

    if result.branches.is_empty() {
        writeln!(stdout, "  no branches found")?;
    } else {
        writeln!(
            stdout,
            "  {} {} from {} processed leaves",
            result.branches.len(),
            if result.branches.len() == 1 {
                "branch"
            } else {
                "branches"
            },
            result.leaves_processed
        )?;
        for branch in &result.branches {
            writeln!(
                stdout,
                "  \u{2713} {} ({} {})",
                branch.slug,
                branch.leaf_count,
                if branch.leaf_count == 1 {
                    "leaf"
                } else {
                    "leaves"
                }
            )?;
        }
    }

    if !result.leaves_skipped.is_empty() {
        writeln!(stdout)?;
        writeln!(
            stdout,
            "  \u{26a0} skipped {} {} (unparseable frontmatter):",
            result.leaves_skipped.len(),
            if result.leaves_skipped.len() == 1 {
                "leaf"
            } else {
                "leaves"
            }
        )?;
        for file in &result.leaves_skipped {
            writeln!(stdout, "    - {}", file)?;
        }
    }

    Ok(())
}

// ── dry-run preview rendering ────────────────────────────────────────────────

pub fn render_preview_human<W: Write>(
    preview: &SynthesisPreview,
    stdout: &mut W,
    tree_name: &str,
) -> io::Result<()> {
    if preview.status == "noop" {
        match preview.reason.as_deref() {
            Some("empty_tree") => writeln!(stdout, "{} is empty", tree_name)?,
            Some("single_leaf") => writeln!(stdout, "{} only has 1 leaf", tree_name)?,
            Some(NO_NEW_LEAVES_REASON) => writeln!(stdout, "nothing new to synthesize")?,
            _ => writeln!(stdout, "nothing to preview")?,
        };
        writeln!(stdout, "dry run: no files were written")?;
        write_notifications(&preview.notifications, stdout)?;
        return Ok(());
    }

    render_preview_summary_human(preview, stdout)?;
    writeln!(stdout, "dry run: no files were written")?;
    write_notifications(&preview.notifications, stdout)?;
    Ok(())
}

fn render_preview_summary_human<W: Write>(
    preview: &SynthesisPreview,
    stdout: &mut W,
) -> io::Result<()> {
    if let (Some(mode), Some(model)) = (&preview.mode, Some(&preview.model)) {
        if preview.agent {
            writeln!(
                stdout,
                "preview ({mode:?}) using {model} via agent ({turns} turns, {calls} tool calls)",
                turns = preview.turns,
                calls = preview.tool_calls
            )?;
        } else {
            writeln!(stdout, "preview ({mode:?}) using {model}")?;
        }
    } else {
        writeln!(stdout, "preview")?;
    }

    if preview.branches.is_empty() {
        writeln!(stdout, "  no branches found")?;
    } else {
        writeln!(
            stdout,
            "  {} {} from {} leaves",
            preview.branches.len(),
            if preview.branches.len() == 1 {
                "branch"
            } else {
                "branches"
            },
            preview.leaves_processed
        )?;
        for branch in &preview.branches {
            writeln!(
                stdout,
                "  \u{2713} {} ({} {})",
                branch.slug,
                branch.leaves.len(),
                if branch.leaves.len() == 1 {
                    "leaf"
                } else {
                    "leaves"
                }
            )?;
        }
    }

    if !preview.state_unchanged {
        writeln!(
            stdout,
            "  \u{26a0} state changed during the dry-run; preview may be stale"
        )?;
    }

    if !preview.leaves_skipped.is_empty() {
        writeln!(stdout)?;
        writeln!(
            stdout,
            "  \u{26a0} skipped {} {} (unparseable frontmatter):",
            preview.leaves_skipped.len(),
            if preview.leaves_skipped.len() == 1 {
                "leaf"
            } else {
                "leaves"
            },
        )?;
        for file in &preview.leaves_skipped {
            writeln!(stdout, "    - {}", file)?;
        }
    }

    Ok(())
}
