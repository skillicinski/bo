// ── compile output rendering ──────────────────────────────────────────────────

use std::io::{self, Write};

use super::{BranchResult, CompileResult, NO_NEW_LEAVES_REASON};

fn print_notifications(notifications: &[String]) {
    for note in notifications {
        println!("\u{2192} {}", note);
    }
}

fn write_notifications<W: Write>(notifications: &[String], stdout: &mut W) -> io::Result<()> {
    for note in notifications {
        writeln!(stdout, "\u{2192} {}", note)?;
    }
    Ok(())
}

pub fn render_human<W: Write>(
    result: &CompileResult,
    stdout: &mut W,
    tree_name: &str,
) -> io::Result<()> {
    if result.status == "noop" {
        match result.reason.as_deref() {
            Some("empty_tree") => writeln!(stdout, "{} is empty", tree_name)?,
            Some("single_leaf") => writeln!(stdout, "{} only has 1 leaf", tree_name)?,
            Some(NO_NEW_LEAVES_REASON) => writeln!(stdout, "nothing new to compile")?,
            _ => writeln!(stdout, "compiled: no work to do")?,
        };
        write_notifications(&result.notifications, stdout)?;
        return Ok(());
    }

    render_summary_human(result, stdout)?;
    write_notifications(&result.notifications, stdout)?;
    Ok(())
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

pub(super) fn print_result(result: &CompileResult, tree_name: &str) {
    if result.status == "noop" {
        match result.reason.as_deref() {
            Some("empty_tree") => println!("{} is empty", tree_name),
            Some("single_leaf") => println!("{} only has 1 leaf", tree_name),
            Some(NO_NEW_LEAVES_REASON) => println!("nothing new to compile"),
            _ => println!("compiled: no work to do"),
        }
        print_notifications(&result.notifications);
        return;
    }

    print_summary_parts(
        &result.branches,
        result.leaves_processed,
        &result.leaves_skipped,
    );
    print_notifications(&result.notifications);
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
                "  \u{2713} {} ({} {})",
                b.slug,
                b.leaf_count,
                if b.leaf_count == 1 { "leaf" } else { "leaves" }
            );
        }
    }

    if !leaves_skipped.is_empty() {
        println!();
        println!(
            "  \u{26a0} skipped {} {} (unparseable frontmatter):",
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
