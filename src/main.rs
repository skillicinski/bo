use std::env;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process;

const ADJECTIVES: &[&str] = &[
    "amber", "brisk", "calm", "clever", "crisp", "eager", "gentle", "hidden", "mellow", "quiet",
    "rapid", "silver", "steady", "tidy", "vivid",
];
const NOUNS: &[&str] = &[
    "badger", "cedar", "comet", "falcon", "meadow", "otter", "panda", "quartz", "river", "sparrow",
    "sunset", "thicket", "willow", "wren", "zephyr",
];

fn main() {
    match run(env::args().skip(1)) {
        Ok(path) => println!("seeded at {}", path.display()),
        Err(error) => {
            eprintln!("seeding failed: {error}");
            process::exit(1);
        }
    }
}

fn run(mut args: impl Iterator<Item = String>) -> Result<PathBuf, String> {
    if args.next().as_deref() != Some("seed") {
        return Err("usage: bo seed [--name <name>]".to_string());
    }

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

    let home = env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .or_else(|| env::var_os("USERPROFILE").filter(|value| !value.is_empty()))
        .map(PathBuf::from)
        .ok_or_else(|| "HOME or USERPROFILE is not set".to_string())?;

    seed_at(&home, name.as_deref())
}

fn seed_at(home: &Path, requested_name: Option<&str>) -> Result<PathBuf, String> {
    let name = match requested_name {
        Some(name) => name.to_owned(),
        None => random_name().map_err(|error| error.to_string())?,
    };
    validate_name(&name)?;

    let root = home.join(".bo");
    fs::create_dir_all(&root).map_err(|error| error.to_string())?;
    let path = root.join(&name);
    fs::create_dir(&path).map_err(|error| error.to_string())?;
    Ok(path)
}

fn validate_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("name must not be empty".to_string());
    }
    if name == "." || name == ".." {
        return Err("name must be a single directory component".to_string());
    }
    if name.ends_with('.') || name.ends_with(' ') {
        return Err("name must not end with a dot or space".to_string());
    }
    if name.chars().any(|character| {
        character.is_control()
            || matches!(
                character,
                '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'
            )
    }) {
        return Err("name contains an invalid character".to_string());
    }
    if is_reserved_device_name(name) {
        return Err("name is reserved on Windows".to_string());
    }
    Ok(())
}

fn is_reserved_device_name(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or_default();
    let bytes = stem.as_bytes();
    stem.eq_ignore_ascii_case("CON")
        || stem.eq_ignore_ascii_case("PRN")
        || stem.eq_ignore_ascii_case("AUX")
        || stem.eq_ignore_ascii_case("NUL")
        || (bytes.len() == 4
            && (bytes[..3].eq_ignore_ascii_case(b"COM") || bytes[..3].eq_ignore_ascii_case(b"LPT"))
            && bytes[3].is_ascii_digit()
            && bytes[3] != b'0')
}

fn random_name() -> io::Result<String> {
    let mut bytes = [0; 2];
    File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(format!(
        "{}-{}",
        ADJECTIVES[usize::from(bytes[0]) % ADJECTIVES.len()],
        NOUNS[usize::from(bytes[1]) % NOUNS.len()]
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[track_caller]
    fn check_name(name: &str, expected_valid: bool) {
        assert_eq!(
            validate_name(name).is_ok(),
            expected_valid,
            "name: {name:?}"
        );
    }

    #[test]
    fn generated_names_are_lowercase_adjective_noun_names() {
        let name = random_name().unwrap();

        assert!(name
            .chars()
            .all(|character| character.is_ascii_lowercase() || character == '-'));
        assert_eq!(name.matches('-').count(), 1);
        assert!(validate_name(&name).is_ok());
    }

    #[test]
    fn names_are_validated_as_portable_components() {
        for name in [
            "",
            ".",
            "..",
            "../escape",
            "sub/name",
            "sub\\name",
            "CON",
            "CON.txt",
            "COM1",
            "LPT9",
            "a:b",
            "a*b",
            "a?b",
            "a\"b",
            "a<b",
            "a>b",
            "a|b",
            "name.",
            "name ",
            "line\nbreak",
        ] {
            check_name(name, false);
        }

        for name in ["test", "hello-world", "has space", ".hidden", "café"] {
            check_name(name, true);
        }
    }
}
