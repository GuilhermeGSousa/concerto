use std::path::{Path, PathBuf};

use chacha20poly1305::aead::{Aead, AeadCore, KeyInit, OsRng};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};

use crate::case::{CaseFiles, FILES};

const MAGIC: &[u8; 5] = b"OMNS1";
const NONCE_LENGTH: usize = 12;
const COMPRESSION: i32 = 19;

/// The key a sealed case is locked with; it lives in a file outside version control.
#[derive(Clone, PartialEq, Eq)]
pub struct CaseKey([u8; 32]);

impl CaseKey {
    /// A new random key.
    pub fn generate() -> Self {
        Self(ChaCha20Poly1305::generate_key(&mut OsRng).into())
    }

    /// Reads a key written by [`CaseKey::to_hex`]; surrounding whitespace is ignored.
    pub fn from_hex(text: &str) -> Option<Self> {
        let text = text.trim();
        if text.len() != 64 || !text.is_ascii() {
            return None;
        }
        let mut key = [0u8; 32];
        for (byte, pair) in key.iter_mut().zip(text.as_bytes().chunks(2)) {
            *byte = u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()?;
        }
        Some(Self(key))
    }

    /// The key as 64 hexadecimal digits.
    pub fn to_hex(&self) -> String {
        self.0.iter().map(|byte| format!("{byte:02x}")).collect()
    }
}

/// Why a sealed case did not open. It never says anything about what is inside.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SealError {
    /// The bytes are not a sealed case of this version.
    NotABundle,
    /// The key is not the one it was sealed with, or the bundle was changed.
    WrongKeyOrDamaged,
    /// It opened, and what is inside is not a set of case files.
    Unreadable,
    /// The bundle or the key file could not be read.
    Missing,
}

impl std::fmt::Display for SealError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            SealError::NotABundle => "not a sealed case",
            SealError::WrongKeyOrDamaged => "the key does not open it, or it was changed",
            SealError::Unreadable => "it opened, and does not hold case files",
            SealError::Missing => "the bundle or its key could not be read",
        })
    }
}

/// Seals the case files: compressed with zstd, then encrypted with ChaCha20-Poly1305 under `key` and a fresh nonce.
pub fn seal(files: &CaseFiles, key: &CaseKey) -> Vec<u8> {
    let mut archive = Vec::new();
    for name in FILES {
        let Some(text) = files.text(name) else {
            continue;
        };
        for part in [name.as_bytes(), text.as_bytes()] {
            archive.extend_from_slice(&(part.len() as u32).to_le_bytes());
            archive.extend_from_slice(part);
        }
    }
    let packed = zstd::encode_all(archive.as_slice(), COMPRESSION)
        .expect("compressing into memory does not fail");
    let cipher = ChaCha20Poly1305::new(Key::from_slice(&key.0));
    let nonce = ChaCha20Poly1305::generate_nonce(&mut OsRng);
    let locked = cipher
        .encrypt(&nonce, packed.as_slice())
        .expect("a case is far under the cipher's size limit");
    let mut bundle = MAGIC.to_vec();
    bundle.extend_from_slice(&nonce);
    bundle.extend_from_slice(&locked);
    bundle
}

/// Opens a sealed case in memory; nothing is written anywhere.
pub fn open(bundle: &[u8], key: &CaseKey) -> Result<CaseFiles, SealError> {
    let body = bundle
        .strip_prefix(MAGIC.as_slice())
        .filter(|body| body.len() > NONCE_LENGTH)
        .ok_or(SealError::NotABundle)?;
    let (nonce, locked) = body.split_at(NONCE_LENGTH);
    let packed = ChaCha20Poly1305::new(Key::from_slice(&key.0))
        .decrypt(Nonce::from_slice(nonce), locked)
        .map_err(|_| SealError::WrongKeyOrDamaged)?;
    let archive = zstd::decode_all(packed.as_slice()).map_err(|_| SealError::Unreadable)?;
    let mut rest = archive.as_slice();
    let mut take = || -> Result<String, SealError> {
        let (length, after) = rest.split_first_chunk::<4>().ok_or(SealError::Unreadable)?;
        let length = u32::from_le_bytes(*length) as usize;
        if after.len() < length {
            return Err(SealError::Unreadable);
        }
        let (part, after) = after.split_at(length);
        rest = after;
        String::from_utf8(part.to_vec()).map_err(|_| SealError::Unreadable)
    };
    let mut files = CaseFiles::new();
    while let Ok(name) = take() {
        files = files.with_file(name, take()?);
    }
    Ok(files)
}

/// Where the sealed case is looked for: `ONE_MORE_NIGHT_CASE`, or `sealed/case.sealed` under the working directory.
pub fn bundle_path() -> PathBuf {
    std::env::var_os("ONE_MORE_NIGHT_CASE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("sealed/case.sealed"))
}

/// Where its key is looked for: `ONE_MORE_NIGHT_KEY`, or `sealed/case.key` under the working directory.
pub fn key_path() -> PathBuf {
    std::env::var_os("ONE_MORE_NIGHT_KEY")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("sealed/case.key"))
}

/// Opens the bundle at `bundle` with the key in the file at `key`.
pub fn open_files(bundle: &Path, key: &Path) -> Result<CaseFiles, SealError> {
    let key = std::fs::read_to_string(key)
        .ok()
        .and_then(|text| CaseKey::from_hex(&text))
        .ok_or(SealError::Missing)?;
    let bundle = std::fs::read(bundle).map_err(|_| SealError::Missing)?;
    open(&bundle, &key)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{CaseKey, SealError, open, seal};
    use crate::case::{CaseFiles, FILES};

    #[test]
    fn a_sealed_case_opens_with_its_key_alone_and_shows_nothing_without_it() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("cases/test");
        let files = CaseFiles::read_dir(&dir).unwrap();
        let key = CaseKey::generate();
        let bundle = seal(&files, &key);

        let opened = open(&bundle, &key).unwrap();
        for name in FILES {
            assert_eq!(
                opened.text(name),
                files.text(name),
                "{name} comes back whole"
            );
        }
        let plain = files.text("strings").unwrap();
        let shown = plain.lines().nth(1).unwrap().as_bytes();
        assert!(
            !bundle.windows(shown.len()).any(|window| window == shown),
            "no line of the case is readable in the bundle"
        );
        assert_ne!(
            seal(&files, &key),
            bundle,
            "each sealing uses a fresh nonce"
        );
        assert!(CaseKey::from_hex(&key.to_hex()) == Some(key.clone()));

        struct Attempt {
            bundle: Vec<u8>,
            key: CaseKey,
            fails_as: SealError,
        }
        let mut changed = bundle.clone();
        *changed.last_mut().unwrap() ^= 1;
        let attempts = [
            Attempt {
                bundle: bundle.clone(),
                key: CaseKey::generate(),
                fails_as: SealError::WrongKeyOrDamaged,
            },
            Attempt {
                bundle: changed,
                key: key.clone(),
                fails_as: SealError::WrongKeyOrDamaged,
            },
            Attempt {
                bundle: plain.as_bytes().to_vec(),
                key: key.clone(),
                fails_as: SealError::NotABundle,
            },
        ];
        for attempt in attempts {
            assert_eq!(
                open(&attempt.bundle, &attempt.key).err(),
                Some(attempt.fails_as)
            );
        }
    }
}
