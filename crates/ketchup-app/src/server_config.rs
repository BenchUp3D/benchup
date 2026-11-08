//! Deployment configuration, deliberately separate from saved user preferences.
#[cfg(not(target_arch = "wasm32"))]
use std::path::{Path, PathBuf};

pub const KEY: &str = "KETCHUP_SERVER_URL";

#[derive(Debug)]
pub enum ConfigError {
    InvalidValue,
    Unconfigured,
    #[cfg(any(not(target_arch = "wasm32"), test))]
    MissingEquals(usize),
    #[cfg(any(not(target_arch = "wasm32"), test))]
    UnclosedQuote(usize),
    #[cfg(any(not(target_arch = "wasm32"), test))]
    TrailingText(usize),
    #[cfg(not(target_arch = "wasm32"))]
    MissingArgument,
    #[cfg(not(target_arch = "wasm32"))]
    InvalidUnicode,
    #[cfg(not(target_arch = "wasm32"))]
    InFile(PathBuf, Box<ConfigError>),
    #[cfg(not(target_arch = "wasm32"))]
    ReadFile(PathBuf, std::io::Error),
}

#[cfg(any(not(target_arch = "wasm32"), test))]
pub fn dotenv(contents: &str) -> Result<Option<String>, ConfigError> {
    let mut result = None;
    for (index, line) in contents.lines().enumerate() {
        let line = line.trim().trim_start_matches('\u{feff}');
        let line = line.strip_prefix("export ").unwrap_or(line).trim();
        let Some((key, raw)) = line.split_once('=') else {
            if line == KEY {
                return Err(ConfigError::MissingEquals(index + 1));
            }
            continue;
        };
        if key.trim() != KEY {
            continue;
        }
        let raw = raw.trim();
        let value = if raw.starts_with(['\'', '"']) {
            let quote = raw.chars().next().unwrap();
            let end = raw[1..]
                .find(quote)
                .map(|i| i + 1)
                .ok_or(ConfigError::UnclosedQuote(index + 1))?;
            let trailing = raw[end + 1..].trim();
            if !trailing.is_empty() && !trailing.starts_with('#') {
                return Err(ConfigError::TrailingText(index + 1));
            }
            &raw[1..end]
        } else {
            raw.split_once(" #").map_or(raw, |(value, _)| value).trim()
        };
        result = Some(nonempty(value)?);
    }
    Ok(result)
}
#[cfg(not(target_arch = "wasm32"))]
pub fn read_dotenv(path: &Path) -> Result<Option<String>, ConfigError> {
    match std::fs::read_to_string(path) {
        Ok(contents) => {
            dotenv(&contents).map_err(|error| ConfigError::InFile(path.to_owned(), Box::new(error)))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(ConfigError::ReadFile(path.to_owned(), error)),
    }
}
pub fn nonempty(value: &str) -> Result<String, ConfigError> {
    let value = value.trim().trim_end_matches('/');
    if value.is_empty() || value.contains(['\n', '\r']) {
        return Err(ConfigError::InvalidValue);
    }
    Ok(value.into())
}

#[cfg(not(target_arch = "wasm32"))]
pub fn command_line(args: impl IntoIterator<Item = String>) -> Result<Option<String>, ConfigError> {
    let mut args = args.into_iter();
    let mut value = None;
    while let Some(arg) = args.next() {
        if arg == "--server-url" {
            value = Some(nonempty(
                &args
                    .next()
                    .filter(|arg| !arg.starts_with("--"))
                    .ok_or(ConfigError::MissingArgument)?,
            )?);
        } else if let Some(url) = arg.strip_prefix("--server-url=") {
            value = Some(nonempty(url)?);
        }
    }
    Ok(value)
}

pub fn configured_server() -> Result<String, ConfigError> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        if let Some(value) = command_line(std::env::args().skip(1))? {
            return Ok(value);
        }
        if let Some(value) = std::env::var_os(KEY) {
            return nonempty(
                &value
                    .into_string()
                    .map_err(|_| ConfigError::InvalidUnicode)?,
            );
        }
        if let Some(value) = read_dotenv(Path::new(".env"))? {
            return Ok(value);
        }
    }
    nonempty(option_env!("KETCHUP_SERVER_URL").ok_or(ConfigError::Unconfigured)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dotenv_supports_plain_quoted_exported_and_missing_values() {
        for source in [
            "KETCHUP_SERVER_URL=https://example.com/",
            "export KETCHUP_SERVER_URL = 'https://example.com/' # comment",
            "KETCHUP_SERVER_URL=\"https://example.com\"",
            "OTHER=value\nKETCHUP_SERVER_URL=https://example.com # comment",
        ] {
            assert_eq!(
                dotenv(source).unwrap().as_deref(),
                Some("https://example.com")
            );
        }
        assert_eq!(dotenv("# comment\nOTHER=value").unwrap(), None);
        for source in [
            "KETCHUP_SERVER_URL=",
            "KETCHUP_SERVER_URL",
            "KETCHUP_SERVER_URL='unclosed",
            "KETCHUP_SERVER_URL='url' extra",
        ] {
            assert!(dotenv(source).is_err());
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn launch_argument_forms_and_errors() {
        for args in [
            vec!["--server-url", "https://example.com/"],
            vec!["--server-url=https://example.com"],
        ] {
            assert_eq!(
                command_line(args.into_iter().map(str::to_owned))
                    .unwrap()
                    .as_deref(),
                Some("https://example.com")
            );
        }
        for args in [
            vec!["--server-url"],
            vec!["--server-url="],
            vec!["--server-url", "--other"],
        ] {
            assert!(command_line(args.into_iter().map(str::to_owned)).is_err());
        }
        assert_eq!(command_line(Vec::<String>::new()).unwrap(), None);
    }
}
