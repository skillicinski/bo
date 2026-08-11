use std::env;
use std::process;

fn main() {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        Some("seed") => match parse_seed(args) {
            Ok(path) => println!("seeded at {}", path.display()),
            Err(error) => {
                eprintln!("seeding failed: {error}");
                process::exit(1);
            }
        },
        Some("snap") => match parse_snap(args) {
            Ok(report) => {
                if print_snap_report(report.outcomes, None) {
                    process::exit(1);
                }
            }
            Err(mut error) => {
                let reportable = error.source_url.is_some() || !error.completed.is_empty();
                if reportable {
                    let completed = std::mem::take(&mut error.completed);
                    print_snap_report(completed, Some(error));
                } else {
                    eprintln!("snap failed: {error}");
                }
                process::exit(1);
            }
        },
        Some("state") => match parse_state(args) {
            Ok(()) => {}
            Err(error) => {
                eprintln!("state failed: {error}");
                process::exit(1);
            }
        },
        Some("agent") => match bo::run_agent(args) {
            Ok(()) => {}
            Err(error) => {
                eprintln!("agent failed: {error}");
                process::exit(1);
            }
        },
        _ => {
            eprintln!(
                "usage: bo seed [--name <name>] | bo snap <dir> <url>... | bo state <name> [--full] | bo agent <dir> [options]"
            );
            process::exit(1);
        }
    }
}

fn print_snap_report(
    outcomes: Vec<bo::application::SnapOutcome>,
    fatal: Option<bo::application::SnapCommandError>,
) -> bool {
    let total = outcomes.len();
    let failed = outcomes
        .iter()
        .filter(|outcome| outcome.result.is_err())
        .count();
    let aborted = fatal.is_some();
    for outcome in outcomes {
        match outcome.result {
            Ok(filename) => println!("snapped: {} -> {filename}", outcome.source_url),
            Err(error) => eprintln!("failed: {} ({error})", outcome.source_url),
        }
    }
    if let Some(error) = fatal {
        match error.source_url {
            Some(source_url) => eprintln!("failed: {source_url} ({})", error.error),
            None => eprintln!("snap failed: {}", error.error),
        }
    }
    let total_failed = failed + if aborted { 1 } else { 0 };
    let succeeded = total - failed;
    if aborted {
        eprintln!("{succeeded} succeeded / {total_failed} failed; batch aborted");
    } else {
        eprintln!("{succeeded} succeeded / {total_failed} failed");
    }
    aborted || failed > 0
}

fn parse_seed(mut args: impl Iterator<Item = String>) -> Result<std::path::PathBuf, String> {
    let mut name = None;
    while let Some(argument) = args.next() {
        if argument != "--name" || name.is_some() {
            return Err("usage: bo seed [--name <name>]".to_string());
        }
        name = Some(
            args.next()
                .ok_or_else(|| "missing value for --name".to_string())?,
        );
    }

    bo::application::seed(name.as_deref())
}

fn parse_state(mut args: impl Iterator<Item = String>) -> Result<(), String> {
    let name = args
        .next()
        .filter(|name| !name.starts_with('-'))
        .ok_or_else(|| "usage: bo state <name> [--full]".to_string())?;
    let full = match args.next().as_deref() {
        None => false,
        Some("--full") if args.next().is_none() => true,
        Some(_) => return Err("usage: bo state <name> [--full]".to_string()),
    };

    println!("{}", bo::application::state(&name, full)?);
    Ok(())
}

fn parse_snap(
    mut args: impl Iterator<Item = String>,
) -> Result<bo::application::SnapReport, bo::application::SnapCommandError> {
    let name = args
        .next()
        .ok_or_else(|| bo::application::SnapCommandError::input("usage: bo snap <dir> <url>..."))?;
    let urls: Vec<_> = args.collect();
    if urls.is_empty() {
        return Err(bo::application::SnapCommandError::input(
            "usage: bo snap <dir> <url>...",
        ));
    }

    bo::application::snap(&name, &urls)
}
