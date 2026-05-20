// ── compile output rendering ──────────────────────────────────────────────────

use std::io::{self, Write};

use super::{BranchResult, CompileResult, NO_NEW_LEAVES_REASON};

pub fn render_human<W: Write>(result: &CompileResult, stdout: &mut W) -> io::Result<()> {
    if result.status == "noop" {
        match result.reason.as_deref() {
            Some("empty_tree") => writeln!(stdout, "bo is empty!"),
            Some("single_leaf") => writeln!(stdout, "bo only has 1 leaf!"),
            Some(NO_NEW_LEAVES_REASON) => writeln!(stdout, "nothing new to compile"),
            _ => writeln!(stdout, "compiled: no work to do"),
        }?;
        return Ok(());
    }

    render_summary_human(result, stdout)
}

fn render_summary_human<W: Write>(result: &CompileResult, stdout: &mut W) -> io::Result<()> {
    if let (Some(mode), Some(model)) = (&result.mode, &result.model) {
        let ctx = result
            .context_mode
            .as_ref()
            .map(|c| format!(", {c:?}"))
            .unwrap_or_default();
        writeln!(stdout, "compiled ({mode:?}{ctx}) using {model}")?;
    } else {
        writeln!(stdout, "compiled")?;
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
                "  ✓ {} ({} {})",
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
            "  ⚠ skipped {} {} (unparseable frontmatter):",
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

pub(super) fn print_result(result: &CompileResult) {
    if result.status == "noop" {
        match result.reason.as_deref() {
            Some("empty_tree") => println!("bo is empty!"),
            Some("single_leaf") => println!("bo only has 1 leaf!"),
            Some(NO_NEW_LEAVES_REASON) => println!("nothing new to compile"),
            _ => println!("compiled: no work to do"),
        }
        return;
    }

    print_summary_parts(
        &result.branches,
        result.leaves_processed,
        &result.leaves_skipped,
    );
}

fn print_summary_parts(
    branches: &[BranchResult],
    leaves_processed: usize,
    leaves_skipped: &[String],
) {
    if branches.is_empty() {
        println!("compiled: no branches found");
    } else {
        println!(
            "compiled: {} {} from {} processed leaves",
            branches.len(),
            if branches.len() == 1 {
                "branch"
            } else {
                "branches"
            },
            leaves_processed
        );
        for b in branches {
            println!(
                "  ✓ {} ({} {})",
                b.slug,
                b.leaf_count,
                if b.leaf_count == 1 { "leaf" } else { "leaves" }
            );
        }
    }

    if !leaves_skipped.is_empty() {
        println!();
        println!(
            "  ⚠ skipped {} {} (unparseable frontmatter):",
            leaves_skipped.len(),
            if leaves_skipped.len() == 1 {
                "leaf"
            } else {
                "leaves"
            }
        );
        for f in leaves_skipped {
            println!("    - {}", f);
        }
    }
}
