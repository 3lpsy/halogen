use regex::Regex;
use std::sync::LazyLock;
pub static ALPHA_DASH: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-zA-Z0-9._-]+$").unwrap());

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alpha_dash_accepts_alnum_dot_dash_underscore() {
        for s in [
            "a.b-c_1", "abc", "ABC123", "a_b", "a-b", "a.b", "_", "-", ".",
        ] {
            assert!(ALPHA_DASH.is_match(s), "{s:?} should match");
        }
    }

    #[test]
    fn alpha_dash_rejects_spaces_slashes_and_other_chars() {
        for s in ["a b", "a/b", "a\\b", "a+b", "a@b", "a!b", "café"] {
            assert!(!ALPHA_DASH.is_match(s), "{s:?} should NOT match");
        }
    }

    #[test]
    fn alpha_dash_rejects_empty() {
        // The `+` quantifier requires at least one char.
        assert!(!ALPHA_DASH.is_match(""));
    }
}
