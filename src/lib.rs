#[macro_use]
extern crate log;

pub mod db;
pub mod feed;
pub mod mastodon;
pub mod models;
// pub mod schema;
pub mod crawler;
mod json_feed;
pub mod social_network;
#[cfg(feature = "twitter")]
pub mod twitter;

use std::env::VarError;
use std::ffi::OsStr;
use std::str::FromStr;
use std::time::Duration;
use std::{env, fmt};

pub fn env_var<K: AsRef<OsStr>>(key: K) -> Result<String, ErrorMessage> {
    env::var(&key).map_err(|err| match err {
        VarError::NotPresent => ErrorMessage(format!(
            "environment variable '{}' is not set",
            key.as_ref().to_string_lossy()
        )),
        VarError::NotUnicode(_) => ErrorMessage(format!(
            "environment variable '{}' is not valid UTF-8",
            key.as_ref().to_string_lossy()
        )),
    })
}

#[derive(Debug, Eq, PartialEq)]
pub struct Delay(Duration);

#[derive(Debug)]
pub struct ErrorMessage(pub String);

impl fmt::Display for ErrorMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl std::error::Error for ErrorMessage {}

impl Delay {
    pub fn from_secs(secs: u16) -> Self {
        Delay(Duration::from_secs(secs.into()))
    }
}

impl FromStr for Delay {
    type Err = ErrorMessage;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let raw = s.as_bytes();
        match raw.last().copied() {
            Some(c) if c == b's' || c == b'm' => {
                let len = s.len() - 1;
                let num_str = unsafe { std::str::from_utf8_unchecked(&raw[0..len]) };
                let value: u16 = num_str
                    .parse()
                    .map_err(|_| ErrorMessage(format!("{s} is not a valid delay")))?;
                let seconds = if c == b'm' {
                    value
                        .checked_mul(60)
                        .ok_or_else(|| ErrorMessage(format!("{s} is too big")))?
                } else {
                    value
                };
                Ok(Delay(Duration::from_secs(seconds.into())))
            }
            Some(_) | None => Err(ErrorMessage("delay must end with 's' or 'm'".into())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_delay_ok() {
        let d: Delay = "10s".parse().unwrap();
        assert_eq!(d, Delay(Duration::from_secs(10)));

        let d: Delay = "10m".parse().unwrap();
        assert_eq!(d, Delay(Duration::from_secs(10 * 60)));
    }

    #[test]
    fn test_parse_delay_err() {
        let d = "10".parse::<Delay>().unwrap_err();
        assert_eq!(d, "delay must end with 's' or 'm'");

        let d = "10h".parse::<Delay>().unwrap_err();
        assert_eq!(d, "delay must end with 's' or 'm'");

        let d = "-10s".parse::<Delay>().unwrap_err();
        assert_eq!(d, "-10s is not a valid delay");

        let d = "10ss".parse::<Delay>().unwrap_err();
        assert_eq!(d, "10ss is not a valid delay");

        let d = "1000000s".parse::<Delay>().unwrap_err();
        assert_eq!(d, "1000000s is not a valid delay");

        let d = "2000m".parse::<Delay>().unwrap_err();
        assert_eq!(d, "2000m is too big");
    }
}
