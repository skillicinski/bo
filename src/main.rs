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
            Ok(0) => {}
            Ok(_) => process::exit(1),
            Err(error) => {
                eprintln!("snap failed: {error}");
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

fn parse_snap(mut args: impl Iterator<Item = String>) -> Result<usize, String> {
    let name = args
        .next()
        .ok_or_else(|| "usage: bo snap <dir> <url>...".to_string())?;
    let urls: Vec<_> = args.collect();
    if urls.is_empty() {
        return Err("usage: bo snap <dir> <url>...".to_string());
    }

    let report = bo::application::snap(&name, &urls)?;
    let failed = report
        .outcomes
        .iter()
        .filter(|(_, result)| result.is_err())
        .count();
    for (url, result) in report.outcomes {
        match result {
            Ok(filename) => println!("snapped: {url} -> {filename}"),
            Err(error) => println!("failed: {url} ({error})"),
        }
    }
    println!("{} succeeded / {failed} failed", urls.len() - failed);
    Ok(failed)
}
