//! The user key, its 24 words and device certificates, against the vectors
//! in `tests/vectors`.

use identity::certificate::{self, CERTIFICATE_LEN, CERTIFICATE_VERSION, Certificate, MAX_CHAIN};
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
    assert_ne!(user.0, device(&seed).0);
}

fn device(seed: &[u8; 32]) -> DeviceId {
    DeviceId(keys::signing_key(seed).verifying_key().to_bytes())
}

#[test]
fn the_certificates_and_the_chain_are_those_of_the_vector() {
    let vectors = vectors(CERTIFICATE);
    assert_eq!(vectors.len(), 5);
    let user = Seed::from_bytes(hex32(vectors[0][0])).user_key();
    let seeds: Vec<[u8; 32]> = vectors[1..4].iter().map(|v| hex32(v[0])).collect();
    let mut certificates = Vec::new();
    for (at, vector) in vectors[1..4].iter().enumerate() {
        let id = device(&seeds[at]);
        assert_eq!(id, DeviceId(hex32(vector[1])));
        let bytes = match at {
            0 => user.certify(id),
            _ => certificate::vouch(&keys::signing_key(&seeds[at - 1]), user.id(), id),
        };
        assert_eq!(bytes, Ok(hex(vector[2])));
        let bytes = bytes.unwrap();
        assert_eq!(bytes.len(), CERTIFICATE_LEN);
        assert_eq!(bytes[0], CERTIFICATE_VERSION);
        let opened = Certificate::open(&bytes).unwrap();
        assert_eq!((opened.user, opened.device), (user.id(), id));
        assert_eq!(opened.by_user(), at == 0);
        certificates.push(bytes);
    }
    let chain = certificate::chain(&certificates).unwrap();
    assert_eq!(chain, hex(vectors[4][0]));
    let verified = certificate::verify(&chain, user.id(), device(&seeds[2])).unwrap();
    assert_eq!(verified.len(), 3);
}

/// A user and `n` devices, each vouched for by the one before it.
struct Person {
    user: UserId,
    seeds: Vec<[u8; 32]>,
    certificates: Vec<Vec<u8>>,
}

impl Person {
    fn new(n: usize) -> Self {
        let key = Seed::generate().user_key();
        let seeds: Vec<[u8; 32]> = (0..n).map(|_| keys::new_seed()).collect();
        let certificates = (0..n)
            .map(|at| match at {
                0 => key.certify(device(&seeds[0])).unwrap(),
                _ => certificate::vouch(
                    &keys::signing_key(&seeds[at - 1]),
                    key.id(),
                    device(&seeds[at]),
                )
                .unwrap(),
            })
            .collect();
        Self {
            user: key.id(),
            seeds,
            certificates,
        }
    }

    fn device(&self, at: usize) -> DeviceId {
        device(&self.seeds[at])
    }

    /// The chain down to device `at`.
    fn chain(&self, at: usize) -> Vec<u8> {
        certificate::chain(&self.certificates[..=at]).unwrap()
    }
}

#[test]
fn every_device_of_a_chain_proves_itself_with_its_part() {
    let person = Person::new(4);
    for at in 0..4 {
        let chain = person.chain(at);
        let verified = certificate::verify(&chain, person.user, person.device(at)).unwrap();
        assert_eq!(verified.len(), at + 1);
        assert!(verified[0].by_user());
        assert!(verified[1..].iter().all(|c| !c.by_user()));
    }
}

#[test]
fn a_chain_is_for_one_user_and_one_device() {
    let person = Person::new(3);
    let other = Person::new(3);
    let chain = person.chain(2);
    assert_eq!(
        certificate::verify(&chain, other.user, person.device(2)),
        Err(Error::Mismatch)
    );
    assert_eq!(
        certificate::verify(&chain, person.user, person.device(1)),
        Err(Error::Mismatch)
    );
    assert_eq!(
        certificate::verify(&chain, person.user, other.device(2)),
        Err(Error::Mismatch)
    );
}

#[test]
fn a_chain_with_a_missing_or_foreign_link_is_refused() {
    let person = Person::new(3);
    let last = person.device(2);

    // The link in the middle left out: the last certificate's signer is not
    // the device before it.
    let gap = certificate::chain(&[
        person.certificates[0].clone(),
        person.certificates[2].clone(),
    ])
    .unwrap();
    assert_eq!(
        certificate::verify(&gap, person.user, last),
        Err(Error::Mismatch)
    );

    // Without the user key's certificate.
    let headless = certificate::chain(&person.certificates[1..]).unwrap();
    assert_eq!(
        certificate::verify(&headless, person.user, last),
        Err(Error::Mismatch)
    );

    // Out of order.
    let mut turned = person.certificates.clone();
    turned.swap(0, 1);
    let turned = certificate::chain(&turned).unwrap();
    assert_eq!(
        certificate::verify(&turned, person.user, last),
        Err(Error::Mismatch)
    );

    // A stranger's device vouching under the user's name: the signature is
    // good, the signer is in nobody's chain.
    let stranger = Person::new(1);
    let forged = certificate::vouch(
        &keys::signing_key(&stranger.seeds[0]),
        person.user,
        stranger.device(0),
    );
    assert!(forged.is_err(), "a device does not vouch for itself");
    let outsider = device(&keys::new_seed());
    let forged = certificate::vouch(
        &keys::signing_key(&stranger.seeds[0]),
        person.user,
        outsider,
    )
    .unwrap();
    assert!(Certificate::open(&forged).is_ok());
    let forged = certificate::chain(&[person.certificates[0].clone(), forged]).unwrap();
    assert_eq!(
        certificate::verify(&forged, person.user, outsider),
        Err(Error::Mismatch)
    );
    // And alone, as if the stranger's device were the user key.
    let alone = certificate::chain(&[certificate::vouch(
        &keys::signing_key(&stranger.seeds[0]),
        person.user,
        outsider,
    )
    .unwrap()])
    .unwrap();
    assert_eq!(
        certificate::verify(&alone, person.user, outsider),
        Err(Error::Mismatch)
    );
}

#[test]
fn a_device_comes_once_in_a_chain() {
    // Two devices vouching for each other in a circle.
    let person = Person::new(2);
    let back = certificate::vouch(
        &keys::signing_key(&person.seeds[1]),
        person.user,
        person.device(0),
    )
    .unwrap();
    let circle = certificate::chain(&[
        person.certificates[0].clone(),
        person.certificates[1].clone(),
        back,
    ])
    .unwrap();
    assert_eq!(
        certificate::verify(&circle, person.user, person.device(0)),
        Err(Error::Mismatch)
    );
}

#[test]
fn a_chain_has_one_to_sixteen_certificates() {
    let person = Person::new(MAX_CHAIN + 1);
    assert!(
        certificate::verify(
            &person.chain(MAX_CHAIN - 1),
            person.user,
            person.device(MAX_CHAIN - 1)
        )
        .is_ok()
    );
    assert!(certificate::chain(&person.certificates).is_err());
    assert!(certificate::chain(&[]).is_err());

    // Built by hand: a count the bytes do not hold, none, and too many.
    let mut long = vec![(MAX_CHAIN + 1) as u8];
    long.extend(person.certificates.iter().flatten());
    assert!(matches!(
        certificate::verify(&long, person.user, person.device(MAX_CHAIN)),
        Err(Error::Malformed(_))
    ));
    let mut chain = person.chain(1);
    chain[0] = 3;
    assert!(certificate::verify(&chain, person.user, person.device(1)).is_err());
    chain[0] = 1;
    assert!(certificate::verify(&chain, person.user, person.device(1)).is_err());
    assert!(certificate::verify(&[0], person.user, person.device(0)).is_err());
    assert!(certificate::verify(&[], person.user, person.device(0)).is_err());
}

#[test]
fn a_changed_certificate_is_refused() {
    let person = Person::new(2);
    for bytes in &person.certificates {
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
    }
    assert!(Certificate::open(&[]).is_err());

    // The same inside a chain.
    let chain = person.chain(1);
    for position in 1..chain.len() {
        let mut tampered = chain.clone();
        tampered[position] ^= 1;
        assert!(
            certificate::verify(&tampered, person.user, person.device(1)).is_err(),
            "{position}"
        );
    }
}

#[test]
fn nobody_certifies_itself() {
    let seed = Seed::generate();
    let key = seed.user_key();
    assert!(key.certify(DeviceId(key.id().0)).is_err());
    let own = keys::new_seed();
    assert!(certificate::vouch(&keys::signing_key(&own), key.id(), device(&own)).is_err());
}
