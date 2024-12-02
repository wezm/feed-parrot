use std::collections::HashMap;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::{env, io};

use chrono::{DateTime, Utc};
use eyre::OptionExt;
use mime::Mime;
use redb::Database;
use reqwest::blocking::Client;
use reqwest::header;

use crate::db::{self, AlreadyPosted};
use crate::feed::{Image, NewFeedItem, PostGuid};
use crate::models::Service;
use crate::RmOnDrop;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessMode {
    ReadOnly,
    ReadWrite,
}

pub trait Registration {
    fn register(db: &Database, client: Client) -> eyre::Result<()>;
}

/// A potential post for sending
pub struct PotentialPost {
    pub text: String,
    pub guid: PostGuid,
    pub image: Option<Image>,
}

/// A post that is ready for sending
pub struct ReadyPost {
    text: String,
    guid: PostGuid,
    image: Option<Image>,
    hash: blake3::Hash,
}

/// A post that has been posted
pub struct Posted {
    pub text: String,
    pub guid: PostGuid,
    pub image: Option<Image>,
    pub hash: blake3::Hash,
    pub at: DateTime<Utc>,
}

pub enum ValidationResult {
    Ok(ReadyPost),
    Duplicate(PotentialPost),
    Error(redb::Error),
}

pub struct FetchedImage {
    path: RmOnDrop,
    pub(crate) content_type: Option<Mime>,
}

impl PotentialPost {
    // FIXME: Bind the result to the service
    pub fn validate(self, db: &Database, service: Service) -> ValidationResult {
        // Check that that is a new post
        match db::already_posted(db, service, &self.text) {
            Ok(AlreadyPosted::No(hash)) => ValidationResult::Ok(ReadyPost {
                text: self.text,
                guid: self.guid,
                image: self.image,
                hash,
            }),
            Ok(AlreadyPosted::Yes) => ValidationResult::Duplicate(self),
            Err(err) => ValidationResult::Error(err),
        }
    }
}

impl ReadyPost {
    pub(crate) fn text(&self) -> &str {
        &self.text
    }

    pub(crate) fn image(&self) -> Option<&Image> {
        self.image.as_ref()
    }

    pub(crate) fn hash(&self) -> blake3::Hash {
        self.hash
    }
}

impl From<ReadyPost> for Posted {
    fn from(post: ReadyPost) -> Self {
        Posted {
            text: post.text,
            guid: post.guid,
            image: post.image,
            hash: post.hash,
            at: Utc::now(),
        }
    }
}

pub trait SocialNetwork: Send + Sync {
    fn service(&self) -> Service;

    fn is_writeable(&self) -> bool;

    fn prepare_post(&self, item: &NewFeedItem) -> eyre::Result<PotentialPost>;

    fn publish_post(&self, client: &Client, post: ReadyPost) -> eyre::Result<Posted>;

    fn fetch_image(&self, client: &Client, image: &Image) -> eyre::Result<FetchedImage> {
        let mut rand = [0u8; 8];
        getrandom::getrandom(&mut rand)?;
        let rand = u64::from_le_bytes(rand);

        // Set up the temp file to download to
        let file_name = image
            .url
            .path_segments()
            .ok_or_eyre("no path segments")?
            .last()
            .and_then(|last| {
                let path = Path::new(last);
                let ext = path.extension();
                let mut new_name = path.file_stem()?.to_os_string();
                new_name.push("-");
                new_name.push(format!("{:x}", rand));
                if let Some(ext) = ext {
                    new_name.push(".");
                    new_name.push(ext);
                }
                Some(PathBuf::from(new_name))
            })
            .unwrap_or_else(|| PathBuf::from(format!("feed-parrot-temp-file-{:x}", rand)));
        let path = RmOnDrop::new(env::temp_dir().join(&file_name));
        let mut file = File::create(path.path())?;

        // Download the file
        debug!(
            "downloading image at {} to {}",
            image.url.as_str(),
            path.path().display()
        );
        let mut response = client.get(image.url.as_str()).send()?;

        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|val| {
                let mime_str = std::str::from_utf8(val.as_bytes()).ok()?;
                mime_str.parse::<Mime>().ok()
            });
        io::copy(&mut response, &mut file)?;

        Ok(FetchedImage { path, content_type })
    }
}

impl FetchedImage {
    pub fn path(&self) -> &Path {
        self.path.path()
    }
}

/// Turn tags with spaces and dashes into PascalCase
pub fn process_tags(raw_tags: &[String]) -> Vec<String> {
    raw_tags.iter().map(|tag| process_tag(tag)).collect()
}

static SPECIAL_CASES: LazyLock<HashMap<&'static str, &'static str>> = LazyLock::new(|| {
    IntoIterator::into_iter([
        ("amd", "AMD"),
        ("arm", "ARM"),
        ("cad", "CAD"),
        ("cli", "CLI"),
        ("cpu", "CPU"),
        ("css", "CSS"),
        ("ext2", "ext2"),
        ("ext3", "ext3"),
        ("ext4", "ext4"),
        ("fosdem", "FOSDEM"),
        ("freebsd", "FreeBSD"),
        ("gnu", "GNU"),
        ("gpl", "GPL"),
        ("gpu", "GPU"),
        ("html", "HTML"),
        ("http", "HTTP"),
        ("ibook", "iBook"),
        ("imac", "iMac"),
        ("imap", "IMAP"),
        ("ios", "iOS"),
        ("ipad", "iPad"),
        ("iphone", "iPhone"),
        ("ipod", "iPod"),
        ("javascript", "JavaScript"),
        ("llvm", "LLVM"),
        ("macos", "macOS"),
        ("netbsd", "NetBSD"),
        ("node-js", "NodeJS"),
        ("nodejs", "NodeJS"),
        ("oled", "OLED"),
        ("openbsd", "OpenBSD"),
        ("pdf", "PDF"),
        ("php", "PHP"),
        ("powerpc", "PowerPC"),
        ("ppc", "PPC"),
        ("pwa", "PWA"),
        ("qnx", "QNX"),
        ("quicktime", "QuickTime"),
        ("raid", "RAID"),
        ("rfc", "RFC"),
        ("riscv", "RISCV"),
        ("rss", "RSS"),
        ("sql", "SQL"),
        ("tls", "TLS"),
        ("tui", "TUI"),
        ("typescript", "TypeScript"),
        ("uefi", "UEFI"),
        ("unix", "UNIX"),
        ("usb", "USB"),
        ("vpn", "VPN"),
        ("webdev", "WebDev"),
        ("xfs", "XFS"),
        ("zfs", "ZFS"),
    ])
    .collect()
});

fn process_tag(raw_tag: &str) -> String {
    // Special handling for specific tags
    if let Some(replacement) = SPECIAL_CASES.get(raw_tag) {
        return replacement.to_string();
    }

    let split_chars = |c: char| c.is_ascii_punctuation() || c.is_whitespace();
    if !raw_tag.chars().any(split_chars) {
        return ucfirst(raw_tag);
    }

    join_to_string::join(raw_tag.split(split_chars).filter_map(|s| {
        if s.is_empty() {
            None
        } else {
            Some(ucfirst(s))
        }
    }))
    .separator("")
    .to_string()
}

fn ucfirst(input: &str) -> String {
    let mut chars = input.chars();
    match chars.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().chain(chars).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_process_tag() {
        assert_eq!(process_tag("dog-cow"), "DogCow".to_string());
        assert_eq!(process_tag("dog_cow"), "DogCow".to_string());
        assert_eq!(process_tag("dog cow"), "DogCow".to_string());
        assert_eq!(process_tag("dog\u{00A0}cow"), "DogCow".to_string());
        assert_eq!(process_tag("dog/cow"), "DogCow".to_string());
        assert_eq!(process_tag("🦜🦜🦜"), "🦜🦜🦜".to_string());
        assert_eq!(process_tag("東京カメラ部"), "東京カメラ部".to_string());
        assert_eq!(process_tag("dog--cow"), "DogCow".to_string());
        assert_eq!(process_tag("-dog-cow-"), "DogCow".to_string());
        assert_eq!(process_tag("MacOS X"), "MacOSX".to_string());
        assert_eq!(process_tag("system7"), "System7".to_string());
        assert_eq!(process_tag("introdução"), "Introdução".to_string());
        assert_eq!(process_tag("Côtes-d'Armor"), "CôtesDArmor".to_string());
        assert_eq!(process_tag("écalgrain bay"), "ÉcalgrainBay".to_string());
        assert_eq!(process_tag("καῦνος"), "Καῦνος".to_string());
        assert_eq!(process_tag("العربية"), "العربية".to_string());
    }

    #[test]
    fn test_process_tag_special_cases() {
        assert_eq!(process_tag("openbsd"), "OpenBSD".to_string());
        assert_eq!(process_tag("node-js"), "NodeJS".to_string());
        assert_eq!(process_tag("macos"), "macOS".to_string());
    }
}
