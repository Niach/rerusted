//! The user key, its 24 words and device certificates, against the vectors
//! in `tests/vectors`.

use identity::certificate::{CERTIFICATE_LEN, CERTIFICATE_VERSION, Certificate};
use identity::user::{Seed, UserId, WORDS, WordsError};
use identity::{Error, keys};
use teamlog::DeviceId;

fn hex(text: &str) -> Vec<u8> {
    assert!(text.len().is_multiple_of(2), "{text}");
    (0..text.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(&text[at..at + 2], 16).unwrap())
        .collect()
}

fn hex32(text: &str) -> [u8; 32] {
    hex(text).try_into().unwrap()
}

/// The lines of a vector file that are not comments, split at `|`.
fn vectors(file: &'static str) -> Vec<Vec<&'static str>> {
    file.lines()
        .filter(|line| !line.starts_with('#') && !line.is_empty())
        .map(|line| line.split('|').map(str::trim).collect())
        .collect()
}

const USER: &str = include_str!("vectors/user-v1.txt");
const CERTIFICATE: &str = include_str!("vectors/certificate-v1.txt");

#[test]
fn seeds_words_and_user_ids_are_those_of_the_vectors() {
    let vectors = vectors(USER);
    assert_eq!(vectors.len(), 7);
    for vector in vectors {
        let seed = Seed::from_bytes(hex32(vector[0]));
        assert_eq!(seed.phrase(), vector[1]);
        assert_eq!(seed.words().len(), WORDS);
        assert_eq!(Seed::from_words(vector[1]), Ok(seed.clone()));
        assert_eq!(seed.user_key().id(), UserId(hex32(vector[2])));
    }
}

#[test]
fn random_seeds_come_back_from_their_words() {
    for _ in 0..200 {
        let seed = Seed::generate();
        assert_eq!(Seed::from_words(&seed.phrase()), Ok(seed));
    }
}

#[test]
fn words_are_read_whatever_the_spacing_and_the_case() {
    let seed = Seed::from_bytes([7; 32]);
    let typed = seed.phrase().to_uppercase().replace(' ', " \n\t ");
    assert_eq!(Seed::from_words(&format!("  {typed}\n")), Ok(seed));
}

#[test]
fn words_that_are_no_seed_are_refused() {
    let seed = Seed::from_bytes([7; 32]);
    let words = seed.words();

    assert_eq!(Seed::from_words(""), Err(WordsError::Count(0)));
    assert_eq!(
        Seed::from_words(&words[..23].join(" ")),
        Err(WordsError::Count(23))
    );
    assert_eq!(
        Seed::from_words(&format!("{} zoo", seed.phrase())),
        Err(WordsError::Count(25))
    );

    let mut wrong = words;
    wrong[5] = "rerusted";
    assert_eq!(
        Seed::from_words(&wrong.join(" ")),
        Err(WordsError::Unknown(5))
    );

    // Two words swapped: each is in the list, the checksum is not theirs.
    let mut swapped = words;
    swapped.swap(0, 23);
    assert_ne!(swapped, words);
    assert_eq!(
        Seed::from_words(&swapped.join(" ")),
        Err(WordsError::Checksum)
    );
}

#[test]
fn a_seed_does_not_show_in_debug_output() {
    assert_eq!(format!("{:?}", Seed::from_bytes([7; 32])), "Seed(…)");
}

#[test]
fn the_user_key_is_not_a_device_key_of_the_same_seed() {
    let seed = [9; 32];
    let user = Seed::from_bytes(seed).user_key().id();
    let device = keys::signing_key(&seed).verifying_key().to_bytes();
    assert_ne!(user.0, device);
}

#[test]
fn the_certificate_is_that_of_the_vector() {
    let vectors = vectors(CERTIFICATE);
    assert_eq!(vectors.len(), 1);
    for vector in vectors {
        let user = Seed::from_bytes(hex32(vector[0])).user_key();
        let device = DeviceId(
            keys::signing_key(&hex32(vector[1]))
                .verifying_key()
                .to_bytes(),
        );
        assert_eq!(device, DeviceId(hex32(vector[2])));
        let (not_before, not_after) = (vector[3].parse().unwrap(), vector[4].parse().unwrap());
        let bytes = hex(vector[5]);
        assert_eq!(bytes.len(), CERTIFICATE_LEN);
        assert_eq!(bytes[0], CERTIFICATE_VERSION);
        assert_eq!(
            user.certify(device, not_before, not_after),
            Ok(bytes.clone())
        );
        assert_eq!(
            Certificate::open(&bytes),
            Ok(Certificate {
                user: user.id(),
                device,
                not_before,
                not_after
            })
        );
    }
}

fn certified() -> (UserId, DeviceId, Vec<u8>) {
    let user = Seed::generate().user_key();
    let device = DeviceId(
        keys::signing_key(&keys::new_seed())
            .verifying_key()
            .to_bytes(),
    );
    let bytes = user.certify(device, 1000, 2000).unwrap();
    (user.id(), device, bytes)
}

#[test]
fn a_certificate_holds_inside_its_window_only() {
    let (user, device, bytes) = certified();
    let verify = |now| Certificate::verify(&bytes, user, device, now).map(|_| ());
    assert_eq!(verify(999), Err(Error::NotYetValid));
    assert_eq!(verify(1000), Ok(()));
    assert_eq!(verify(1999), Ok(()));
    assert_eq!(verify(2000), Err(Error::Expired));
    assert_eq!(verify(u64::MAX), Err(Error::Expired));
}

#[test]
fn a_certificate_is_for_one_user_and_one_device() {
    let (user, device, bytes) = certified();
    let (other_user, other_device, _) = certified();
    assert!(Certificate::verify(&bytes, user, device, 1500).is_ok());
    assert_eq!(
        Certificate::verify(&bytes, other_user, device, 1500),
        Err(Error::Mismatch)
    );
    assert_eq!(
        Certificate::verify(&bytes, user, other_device, 1500),
        Err(Error::Mismatch)
    );
}

#[test]
fn a_window_without_a_second_is_not_signed() {
    let user = Seed::generate().user_key();
    let device = DeviceId([1; 32]);
    assert!(user.certify(device, 5, 5).is_err());
    assert!(user.certify(device, 6, 5).is_err());
    assert!(user.certify(device, 5, 6).is_ok());
}

#[test]
fn a_changed_certificate_is_refused() {
    let (_, _, bytes) = certified();

    let mut newer = bytes.clone();
    newer[0] += 1;
    assert_eq!(Certificate::open(&newer), Err(Error::UnsupportedVersion(2)));

    // One flipped bit anywhere after the version.
    for position in 1..bytes.len() {
        let mut tampered = bytes.clone();
        tampered[position] ^= 1;
        assert!(
            matches!(
                Certificate::open(&tampered),
                Err(Error::BadSignature | Error::Malformed(_))
            ),
            "{position}"
        );
    }

    let mut longer = bytes.clone();
    longer.push(0);
    assert!(Certificate::open(&longer).is_err());
    assert!(Certificate::open(&bytes[..bytes.len() - 1]).is_err());
    assert!(Certificate::open(&[]).is_err());
}

#[test]
fn another_users_signature_does_not_certify() {
    let (_, device, bytes) = certified();
    let forger = Seed::generate().user_key();
    let forged = forger.certify(device, 1000, 2000).unwrap();
    // The forger's signature under the first user's name.
    let mut mixed = bytes[..81].to_vec();
    mixed.extend_from_slice(&forged[81..]);
    assert_eq!(Certificate::open(&mixed), Err(Error::BadSignature));
}
