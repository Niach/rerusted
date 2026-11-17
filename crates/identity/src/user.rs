//! The user key and its 24 words (#RUST-17).
//!
//! A user is one 32-byte seed. The seed is shown and restored as 24 words,
//! and the key the user signs device certificates with is derived from it:
//!
//! ```text
//! words      = BIP39, English list: the 256 bits of the seed, then the first
//!              8 bits of SHA-256(seed), cut into 24 groups of 11 bits, each
//!              the index of a word
//! user key   = Ed25519, secret = SHA-256("rerusted/user/1" seed)
//! user id    = 32 bytes, the user key's public key
//! ```
//!
//! The seed is the BIP39 entropy itself. BIP39's own second step, which
//! stretches the words into a wallet seed, is not used: the words are
//! another spelling of the 32 bytes and nothing else.
//!
//! Nothing here stores the seed, and nothing prints it: [`Seed`] has no
//! `Debug` that shows its bytes.

use std::fmt;
use std::sync::OnceLock;

use ed25519_dalek::SigningKey;
use sha2::{Digest, Sha256};

use crate::keys;

const USER_CONTEXT: &[u8] = b"rerusted/user/1";

/// The number of words a seed is written as.
pub const WORDS: usize = 24;

/// The BIP39 English list: 2048 words in ascending order.
fn list() -> &'static [&'static str] {
    static LIST: OnceLock<Vec<&'static str>> = OnceLock::new();
    LIST.get_or_init(|| include_str!("english.txt").lines().collect())
}

/// A user's public key.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UserId(pub [u8; 32]);

impl fmt::Debug for UserId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "UserId(")?;
        for byte in &self.0[..4] {
            write!(f, "{byte:02x}")?;
        }
        write!(f, "…)")
    }
}

/// Why 24 words are not a seed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WordsError {
    /// Not 24 words but this many.
    Count(usize),
    /// The word at this position, from 0, is not in the list.
    Unknown(usize),
    /// Every word is in the list, but together they are no seed: one of
    /// them is the wrong word, or they are out of order.
    Checksum,
}

impl fmt::Display for WordsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Count(count) => write!(f, "{count} words instead of {WORDS}"),
            Self::Unknown(at) => write!(f, "word {} is not in the list", at + 1),
            Self::Checksum => write!(f, "the words do not fit together"),
        }
    }
}

impl std::error::Error for WordsError {}

/// The 32 bytes a user is. Whoever holds them is the user.
#[derive(Clone, PartialEq, Eq)]
pub struct Seed([u8; 32]);

impl fmt::Debug for Seed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Seed(…)")
    }
}

impl Seed {
    pub fn generate() -> Self {
        Self(keys::new_seed())
    }

    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// The seed and its checksum byte: 264 bits, 24 groups of 11.
    fn bits(&self) -> [u8; 33] {
        let mut bits = [0u8; 33];
        bits[..32].copy_from_slice(&self.0);
        bits[32] = Sha256::digest(self.0)[0];
        bits
    }

    /// The seed as 24 words.
    pub fn words(&self) -> [&'static str; WORDS] {
        let bits = self.bits();
        let list = list();
        std::array::from_fn(|word| {
            let index = (0..11).fold(0usize, |index, bit| {
                let at = word * 11 + bit;
                index << 1 | usize::from(bits[at / 8] >> (7 - at % 8) & 1)
            });
            list[index]
        })
    }

    /// The 24 words in one line, separated by single spaces.
    pub fn phrase(&self) -> String {
        self.words().join(" ")
    }

    /// Reads 24 words back. Words are separated by any white space and may
    /// be written in capitals; nothing else is forgiven.
    pub fn from_words(phrase: &str) -> Result<Self, WordsError> {
        let words: Vec<String> = phrase
            .split_whitespace()
            .map(str::to_ascii_lowercase)
            .collect();
        if words.len() != WORDS {
            return Err(WordsError::Count(words.len()));
        }
        let list = list();
        let mut bits = [0u8; 33];
        for (word, text) in words.iter().enumerate() {
            let index = list
                .binary_search(&text.as_str())
                .map_err(|_| WordsError::Unknown(word))?;
            for bit in 0..11 {
                if index >> (10 - bit) & 1 == 1 {
                    let at = word * 11 + bit;
                    bits[at / 8] |= 1 << (7 - at % 8);
                }
            }
        }
        let seed = Self(bits[..32].try_into().expect("32 of 33 bytes"));
        if seed.bits()[32] != bits[32] {
            return Err(WordsError::Checksum);
        }
        Ok(seed)
    }

    /// The key this user signs device certificates with.
    pub fn user_key(&self) -> UserKey {
        UserKey(SigningKey::from_bytes(
            &Sha256::new()
                .chain_update(USER_CONTEXT)
                .chain_update(self.0)
                .finalize()
                .into(),
        ))
    }
}

/// A user's signing key. It signs device certificates and nothing else: no
/// log entry is signed with it.
pub struct UserKey(pub(crate) SigningKey);

impl UserKey {
    pub fn id(&self) -> UserId {
        UserId(self.0.verifying_key().to_bytes())
    }
}
