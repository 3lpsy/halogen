use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::str::FromStr;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Platform {
    IOS,
    Android,
    Linux,
    Mac,
    Windows,
}

impl Platform {
    pub fn from_ua(ua: &str) -> Self {
        let ua = ua.to_lowercase();
        if ua.contains("android") {
            Platform::Android
        } else if ua.contains("iphone") || ua.contains("ipad") || ua.contains("ipod") {
            Platform::IOS
        } else if ua.contains("macintosh") || ua.contains("mac os") {
            Platform::Mac
        } else if ua.contains("linux") {
            Platform::Linux
        } else if ua.contains("windows") {
            Platform::Windows
        } else {
            panic!("Unknown platform: {}", ua);
        }
    }
    pub fn is_mobile(&self) -> bool {
        matches!(self, Platform::IOS | Platform::Android)
    }

    pub fn is_desktop(&self) -> bool {
        matches!(self, Platform::Linux | Platform::Mac | Platform::Windows)
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub enum Environment {
    #[default]
    Dev,
    Prod,
    Testing,
}

impl FromStr for Environment {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "dev" => Ok(Environment::Dev),
            "prod" => Ok(Environment::Prod),
            "testing" | "test" => Ok(Environment::Testing),
            _ => Err(anyhow::anyhow!(
                "Invalid environment: '{}'. Use either 'dev' or 'prod'.",
                s
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- Environment::from_str -----------------------------------------------

    #[test]
    fn environment_from_str_all_arms() {
        // Case-insensitive; "test"/"testing" both map to Testing.
        assert_eq!(Environment::from_str("dev").unwrap(), Environment::Dev);
        assert_eq!(Environment::from_str("DEV").unwrap(), Environment::Dev);
        assert_eq!(Environment::from_str("prod").unwrap(), Environment::Prod);
        assert_eq!(Environment::from_str("Prod").unwrap(), Environment::Prod);
        assert_eq!(
            Environment::from_str("testing").unwrap(),
            Environment::Testing
        );
        assert_eq!(Environment::from_str("test").unwrap(), Environment::Testing);
        assert_eq!(Environment::from_str("TEST").unwrap(), Environment::Testing);
    }

    #[test]
    fn environment_from_str_invalid_errors() {
        assert!(Environment::from_str("staging").is_err());
        assert!(Environment::from_str("").is_err());
    }

    #[test]
    fn environment_default_is_dev() {
        assert_eq!(Environment::default(), Environment::Dev);
    }

    // --- Platform::from_ua ---------------------------------------------------

    #[test]
    fn platform_from_ua_per_branch() {
        // android wins; iphone/ipad/ipod => iOS; mac strings => Mac; etc.
        let cases = [
            ("Mozilla/5.0 (Linux; Android 13)", Platform::Android),
            ("Mozilla/5.0 (iPhone; CPU iPhone OS 17_0)", Platform::IOS),
            ("Mozilla/5.0 (iPad; CPU OS 17_0)", Platform::IOS),
            ("Mozilla/5.0 (iPod touch)", Platform::IOS),
            (
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15)",
                Platform::Mac,
            ),
            ("Mozilla/5.0 (X11; Linux x86_64)", Platform::Linux),
            (
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64)",
                Platform::Windows,
            ),
        ];
        for (ua, want) in cases {
            assert_eq!(Platform::from_ua(ua), want, "ua {ua:?}");
        }
    }

    #[test]
    fn platform_from_ua_is_case_insensitive() {
        assert_eq!(Platform::from_ua("ANDROID"), Platform::Android);
        assert_eq!(Platform::from_ua("WINDOWS"), Platform::Windows);
    }

    #[test]
    #[should_panic(expected = "Unknown platform")]
    fn platform_from_ua_unknown_panics() {
        Platform::from_ua("some-unrecognised-bot/1.0");
    }

    // --- is_mobile / is_desktop ----------------------------------------------

    #[test]
    fn platform_mobile_desktop_classification() {
        for p in [Platform::IOS, Platform::Android] {
            assert!(p.is_mobile(), "{p:?} should be mobile");
            assert!(!p.is_desktop(), "{p:?} should not be desktop");
        }
        for p in [Platform::Linux, Platform::Mac, Platform::Windows] {
            assert!(p.is_desktop(), "{p:?} should be desktop");
            assert!(!p.is_mobile(), "{p:?} should not be mobile");
        }
    }
}
