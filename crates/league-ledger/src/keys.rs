//! Ed25519 keys and XRPL addresses.
//!
//! XRPL's ed25519 derivation (xrpl.org, Cryptographic Keys): the 32-byte
//! secret key is the SHA-512Half of the 16-byte seed; the public key is the
//! standard ed25519 public key, prefixed with `0xED` to make 33 bytes. The
//! account ID is RIPEMD-160 of SHA-256 of that 33-byte key.
//!
//! The cryptography is done by maintained crates (`ed25519-dalek`, `sha2`,
//! `ripemd`); this module only arranges bytes. Only ed25519 is supported:
//! secp256k1 is refused on purpose.

use ed25519_dalek::{Signer, SigningKey};
use ripemd::Ripemd160;
use sha2::{Digest, Sha256, Sha512};
use std::fmt;

/// Version bytes of an ed25519 family seed (encodes to a leading `sEd`).
const ED25519_SEED_PREFIX: [u8; 3] = [0x01, 0xE1, 0x4B];
/// Version byte of a classic address (encodes to a leading `r`).
const ACCOUNT_ID_PREFIX: u8 = 0x00;

/// Something wrong with a seed or an address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyError {
    /// Not valid base58 in XRPL's alphabet.
    NotBase58,
    /// The four-byte checksum does not match.
    BadChecksum,
    /// A seed that is not an ed25519 seed (`sEd...`). secp256k1 seeds are
    /// refused: the game uses ed25519 only.
    NotEd25519Seed,
    /// A classic address that does not decode to 20 bytes with prefix 0.
    NotAnAddress,
}

impl fmt::Display for KeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            KeyError::NotBase58 => "not valid XRPL base58",
            KeyError::BadChecksum => "the checksum does not match",
            KeyError::NotEd25519Seed => "not an ed25519 seed (sEd...); only ed25519 is supported",
            KeyError::NotAnAddress => "not a classic XRPL address",
        })
    }
}

impl std::error::Error for KeyError {}

/// An ed25519 key pair for one XRPL account. Never prints its secret.
pub struct Keypair {
    signing: SigningKey,
    public: [u8; 33],
}

impl fmt::Debug for Keypair {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Keypair({}, <secret hidden>)", self.address())
    }
}

impl Keypair {
    /// The key pair for 16 bytes of seed entropy.
    pub fn from_entropy(entropy: &[u8; 16]) -> Self {
        let digest = Sha512::digest(entropy);
        let mut secret = [0u8; 32];
        secret.copy_from_slice(&digest[..32]);
        let signing = SigningKey::from_bytes(&secret);
        let mut public = [0u8; 33];
        public[0] = 0xED;
        public[1..].copy_from_slice(signing.verifying_key().as_bytes());
        Keypair { signing, public }
    }

    /// The key pair for an ed25519 family seed (`sEd...`).
    pub fn from_seed(seed: &str) -> Result<Self, KeyError> {
        let payload = decode_check(seed)?;
        if payload.len() != 19 || payload[..3] != ED25519_SEED_PREFIX {
            return Err(KeyError::NotEd25519Seed);
        }
        let mut entropy = [0u8; 16];
        entropy.copy_from_slice(&payload[3..]);
        Ok(Self::from_entropy(&entropy))
    }

    /// A new key pair from 16 bytes of operating-system randomness, and its
    /// seed. The seed is the only way to recover the account: keep it secret.
    pub fn generate() -> Result<(Self, String), getrandom::Error> {
        let mut entropy = [0u8; 16];
        getrandom::fill(&mut entropy)?;
        Ok((Self::from_entropy(&entropy), encode_seed(&entropy)))
    }

    /// The 33-byte public key, `0xED` first.
    pub fn public_key(&self) -> &[u8; 33] {
        &self.public
    }

    /// The account ID: RIPEMD-160 of SHA-256 of the public key.
    pub fn account_id(&self) -> [u8; 20] {
        account_id_from_public(&self.public)
    }

    /// The classic address (`r...`).
    pub fn address(&self) -> String {
        encode_address(&self.account_id())
    }

    /// Sign `message` (the full signing data, not a hash). Returns the
    /// 64-byte ed25519 signature.
    pub fn sign(&self, message: &[u8]) -> [u8; 64] {
        self.signing.sign(message).to_bytes()
    }
}

/// The account ID for a 33-byte public key.
pub fn account_id_from_public(public: &[u8; 33]) -> [u8; 20] {
    let inner = Sha256::digest(public);
    let outer = Ripemd160::digest(inner);
    let mut id = [0u8; 20];
    id.copy_from_slice(&outer);
    id
}

/// The classic address for an account ID.
pub fn encode_address(id: &[u8; 20]) -> String {
    let mut payload = Vec::with_capacity(21);
    payload.push(ACCOUNT_ID_PREFIX);
    payload.extend_from_slice(id);
    encode_check(&payload)
}

/// The account ID in a classic address.
pub fn decode_address(address: &str) -> Result<[u8; 20], KeyError> {
    let payload = decode_check(address)?;
    if payload.len() != 21 || payload[0] != ACCOUNT_ID_PREFIX {
        return Err(KeyError::NotAnAddress);
    }
    let mut id = [0u8; 20];
    id.copy_from_slice(&payload[1..]);
    Ok(id)
}

/// The `sEd...` seed for 16 bytes of entropy.
pub fn encode_seed(entropy: &[u8; 16]) -> String {
    let mut payload = Vec::with_capacity(19);
    payload.extend_from_slice(&ED25519_SEED_PREFIX);
    payload.extend_from_slice(entropy);
    encode_check(&payload)
}

fn checksum(payload: &[u8]) -> [u8; 4] {
    let twice = Sha256::digest(Sha256::digest(payload));
    [twice[0], twice[1], twice[2], twice[3]]
}

fn encode_check(payload: &[u8]) -> String {
    let mut data = payload.to_vec();
    data.extend_from_slice(&checksum(payload));
    bs58::encode(data)
        .with_alphabet(bs58::Alphabet::RIPPLE)
        .into_string()
}

fn decode_check(text: &str) -> Result<Vec<u8>, KeyError> {
    let data = bs58::decode(text)
        .with_alphabet(bs58::Alphabet::RIPPLE)
        .into_vec()
        .map_err(|_| KeyError::NotBase58)?;
    if data.len() < 5 {
        return Err(KeyError::BadChecksum);
    }
    let (payload, sum) = data.split_at(data.len() - 4);
    if checksum(payload) != sum {
        return Err(KeyError::BadChecksum);
    }
    Ok(payload.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_seed_round_trips_and_is_ed25519() -> Result<(), KeyError> {
        let entropy = [0x11u8; 16];
        let seed = encode_seed(&entropy);
        assert!(seed.starts_with("sEd"), "{seed}");
        let pair = Keypair::from_seed(&seed)?;
        assert_eq!(pair.public_key()[0], 0xED);
        assert_eq!(pair.address(), Keypair::from_entropy(&entropy).address());
        Ok(())
    }

    #[test]
    fn an_address_round_trips() -> Result<(), KeyError> {
        let id = [0x42u8; 20];
        let address = encode_address(&id);
        assert!(address.starts_with('r'));
        assert_eq!(decode_address(&address)?, id);
        Ok(())
    }

    #[test]
    fn bad_input_is_refused() {
        assert_eq!(decode_address("0OIl").err(), Some(KeyError::NotBase58));
        assert_eq!(decode_address("rrrr").err(), Some(KeyError::BadChecksum));
        let mut address = encode_address(&[1u8; 20]);
        let last = address.pop().unwrap_or('r');
        address.push(if last == 'r' { 'p' } else { 'r' });
        assert_eq!(decode_address(&address).err(), Some(KeyError::BadChecksum));
        // A seed is not an address, and an address is not an ed25519 seed.
        let seed = encode_seed(&[3u8; 16]);
        assert_eq!(decode_address(&seed).err(), Some(KeyError::NotAnAddress));
        let address = encode_address(&[3u8; 20]);
        assert_eq!(
            Keypair::from_seed(&address).err(),
            Some(KeyError::NotEd25519Seed)
        );
        for e in [
            KeyError::NotBase58,
            KeyError::BadChecksum,
            KeyError::NotEd25519Seed,
            KeyError::NotAnAddress,
        ] {
            assert!(!e.to_string().is_empty());
        }
    }

    #[test]
    fn a_keypair_never_prints_its_secret() -> Result<(), getrandom::Error> {
        let (pair, seed) = Keypair::generate()?;
        let shown = format!("{pair:?}");
        assert!(shown.contains(&pair.address()));
        assert!(shown.contains("<secret hidden>"));
        assert!(!shown.contains(&seed));
        Ok(())
    }

    #[test]
    fn a_signature_verifies_with_the_public_key() {
        use ed25519_dalek::{Signature, Verifier, VerifyingKey};
        let pair = Keypair::from_entropy(&[9u8; 16]);
        let message = b"STX\0merchant league";
        let signature = pair.sign(message);
        let mut raw = [0u8; 32];
        raw.copy_from_slice(&pair.public_key()[1..]);
        let key = VerifyingKey::from_bytes(&raw);
        assert!(key.is_ok());
        if let Ok(key) = key {
            assert!(
                key.verify(message, &Signature::from_bytes(&signature))
                    .is_ok()
            );
        }
    }
}
