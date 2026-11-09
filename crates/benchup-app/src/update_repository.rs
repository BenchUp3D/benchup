//! Build-time update source normalization, shared with the regression tests.
pub const REPOSITORY: &str = "BenchUp3D/editor";

pub fn url(value: &str) -> Result<String, String> {
    let value = value.trim().trim_end_matches('/').trim_end_matches(".git");
    let value = value
        .strip_prefix("https://github.com/")
        .or_else(|| value.strip_prefix("git@github.com:"))
        .or_else(|| value.strip_prefix("ssh://git@github.com/"))
        .unwrap_or(value);
    let parts: Vec<_> = value.split('/').collect();
    if parts.len() != 2
        || parts.iter().any(|part| {
            part.is_empty()
                || *part == "."
                || *part == ".."
                || !part
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
        })
    {
        return Err("Set BENCHUP_UPDATE_REPOSITORY to a GitHub owner/repository".into());
    }
    // Accept either order of the organization transfer and repository rename.
    // Unrelated forks must continue to use their own release source.
    let official_owner = ["Ketchup3D", "BenchUp3D"]
        .iter()
        .any(|owner| parts[0].eq_ignore_ascii_case(owner));
    let official_name = ["app", "editor"]
        .iter()
        .any(|name| parts[1].eq_ignore_ascii_case(name));
    let repository = if official_owner && official_name {
        REPOSITORY
    } else {
        value
    };
    Ok(format!("https://github.com/{repository}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transfer_and_rename_in_either_order_resolve_to_the_new_official_feed() {
        for owner in ["Ketchup3D", "BenchUp3D", "ketchup3d", "benchup3d"] {
            for name in ["app", "editor"] {
                for input in [
                    format!("{owner}/{name}"),
                    format!("https://github.com/{owner}/{name}/"),
                    format!("git@github.com:{owner}/{name}.git"),
                    format!("ssh://git@github.com/{owner}/{name}.git"),
                ] {
                    assert_eq!(url(&input).unwrap(), "https://github.com/BenchUp3D/editor");
                }
            }
        }
        assert_eq!(
            url(REPOSITORY).unwrap(),
            "https://github.com/BenchUp3D/editor"
        );
    }

    #[test]
    fn custom_forks_are_preserved_and_invalid_sources_are_rejected() {
        assert_eq!(
            url("someone/editor").unwrap(),
            "https://github.com/someone/editor"
        );
        assert_eq!(
            url("BenchUp3D/other").unwrap(),
            "https://github.com/BenchUp3D/other"
        );
        for input in [
            "",
            "../editor",
            "org/repo/extra",
            "https://example.org/org/repo",
            "org/repo?token=x",
        ] {
            assert!(url(input).is_err());
        }
    }
}
