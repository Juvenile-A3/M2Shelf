use std::fmt::Write as _;

use argon2::{Algorithm, Argon2, Params, Version};
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use chacha20poly1305::{
    aead::{Aead, KeyInit, Payload},
    XChaCha20Poly1305, XNonce,
};
use ed25519_dalek::SigningKey;
use rand_core::{CryptoRng, OsRng, RngCore};
use serde::Serialize;
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

pub const APP_ID: &str = "app.morimediashelf.desktop";
pub const KEY_DIRECTORY_NAME: &str = "M2Shelf-Production-Key";
pub const KEY_FILE_NAME: &str = "encrypted-private-key.m2key";
pub const METADATA_FILE_NAME: &str = "key-metadata.json";
pub const README_FILE_NAME: &str = "README.txt";

const MAGIC: &[u8; 16] = b"M2SHELF-M2KEY\0\0\0";
const FORMAT_VERSION: u16 = 1;
const KDF_ARGON2ID: u8 = 1;
const CIPHER_XCHACHA20_POLY1305: u8 = 1;
const SALT_LEN: usize = 32;
const NONCE_LEN: usize = 24;
const PUBLIC_KEY_LEN: usize = 32;
const SEED_LEN: usize = 32;
const TAG_LEN: usize = 16;
const CIPHERTEXT_LEN: usize = SEED_LEN + TAG_LEN;
const MAX_CONTAINER_BYTES: usize = 1024;
const MAX_KEY_ID_BYTES: usize = 64;

#[cfg(not(test))]
const ARGON2_MEMORY_KIB: u32 = 256 * 1024;
#[cfg(test)]
const ARGON2_MEMORY_KIB: u32 = 8 * 1024;
#[cfg(not(test))]
const ARGON2_ITERATIONS: u32 = 3;
#[cfg(test)]
const ARGON2_ITERATIONS: u32 = 1;
const ARGON2_PARALLELISM: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyMetadata {
    pub schema_version: u32,
    pub app_id: &'static str,
    pub key_id: String,
    pub key_format: &'static str,
    pub public_key: String,
    pub public_key_sha256: String,
}

pub struct DecryptedKey {
    pub seed: Zeroizing<[u8; SEED_LEN]>,
    pub public_key: [u8; PUBLIC_KEY_LEN],
}

struct ParsedContainer<'a> {
    memory_kib: u32,
    iterations: u32,
    parallelism: u32,
    salt: [u8; SALT_LEN],
    nonce: [u8; NONCE_LEN],
    public_key: [u8; PUBLIC_KEY_LEN],
    aad: &'a [u8],
    ciphertext: &'a [u8],
}

pub fn encrypt_seed(
    seed: &[u8; SEED_LEN],
    password: &str,
    key_id: &str,
) -> Result<(Vec<u8>, KeyMetadata), String> {
    encrypt_seed_with_rng(seed, password, key_id, &mut OsRng)
}

fn encrypt_seed_with_rng<R: RngCore + CryptoRng>(
    seed: &[u8; SEED_LEN],
    password: &str,
    key_id: &str,
    rng: &mut R,
) -> Result<(Vec<u8>, KeyMetadata), String> {
    validate_key_id(key_id)?;
    let signing_key = SigningKey::from_bytes(seed);
    let public_key = signing_key.verifying_key().to_bytes();
    drop(signing_key);

    let mut salt = [0_u8; SALT_LEN];
    let mut nonce = [0_u8; NONCE_LEN];
    rng.fill_bytes(&mut salt);
    rng.fill_bytes(&mut nonce);

    let mut encryption_key = Zeroizing::new([0_u8; 32]);
    derive_key(
        password,
        &salt,
        ARGON2_MEMORY_KIB,
        ARGON2_ITERATIONS,
        ARGON2_PARALLELISM,
        &mut encryption_key,
    )?;
    let aad = serialize_header(
        ARGON2_MEMORY_KIB,
        ARGON2_ITERATIONS,
        ARGON2_PARALLELISM,
        &salt,
        &nonce,
        &public_key,
        key_id,
    )?;
    let cipher = XChaCha20Poly1305::new_from_slice(encryption_key.as_ref())
        .map_err(|_| "Could not initialize the USB key cipher.".to_string())?;
    let ciphertext = cipher
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: seed,
                aad: &aad,
            },
        )
        .map_err(|_| "Could not encrypt the USB signing key.".to_string())?;
    if ciphertext.len() != CIPHERTEXT_LEN {
        return Err("The encrypted USB key has an unexpected size.".into());
    }
    let mut output = aad;
    output.extend_from_slice(&ciphertext);
    if output.len() > MAX_CONTAINER_BYTES {
        return Err("The encrypted USB key container is too large.".into());
    }
    Ok((output, metadata(key_id, &public_key)))
}

pub fn decrypt_seed(bytes: &[u8], password: &str) -> Result<DecryptedKey, String> {
    let parsed = parse_container(bytes)?;
    let mut encryption_key = Zeroizing::new([0_u8; 32]);
    derive_key(
        password,
        &parsed.salt,
        parsed.memory_kib,
        parsed.iterations,
        parsed.parallelism,
        &mut encryption_key,
    )?;
    let cipher = XChaCha20Poly1305::new_from_slice(encryption_key.as_ref())
        .map_err(|_| "Could not initialize the USB key cipher.".to_string())?;
    let plaintext = Zeroizing::new(
        cipher
            .decrypt(
                XNonce::from_slice(&parsed.nonce),
                Payload {
                    msg: parsed.ciphertext,
                    aad: parsed.aad,
                },
            )
            .map_err(|_| {
                "The USB key password is incorrect or the key file is damaged.".to_string()
            })?,
    );
    let seed = Zeroizing::new(
        plaintext
            .as_slice()
            .try_into()
            .map_err(|_| "The decrypted Ed25519 seed has an invalid size.".to_string())?,
    );
    let signing_key = SigningKey::from_bytes(&seed);
    let derived_public_key = signing_key.verifying_key().to_bytes();
    drop(signing_key);
    if derived_public_key != parsed.public_key {
        return Err("The decrypted seed does not match the authenticated USB key metadata.".into());
    }
    Ok(DecryptedKey {
        seed,
        public_key: parsed.public_key,
    })
}

pub fn metadata_json(metadata: &KeyMetadata) -> Result<Vec<u8>, String> {
    let mut json = serde_json::to_vec_pretty(metadata)
        .map_err(|_| "Could not serialize the USB key metadata.".to_string())?;
    json.push(b'\n');
    Ok(json)
}

pub fn readme_bytes() -> &'static [u8] {
    b"M2Shelf production update signing key\r\n\
\r\n\
Keep this USB drive offline except while signing a release.\r\n\
The private seed is encrypted and requires the password entered at signing time.\r\n\
Never copy this directory into the source repository or upload it to GitHub.\r\n\
Keep the legacy DPAPI file as a separate offline recovery backup until its retirement is explicitly approved.\r\n"
}

fn serialize_header(
    memory_kib: u32,
    iterations: u32,
    parallelism: u32,
    salt: &[u8; SALT_LEN],
    nonce: &[u8; NONCE_LEN],
    public_key: &[u8; PUBLIC_KEY_LEN],
    key_id: &str,
) -> Result<Vec<u8>, String> {
    validate_key_id(key_id)?;
    let key_id_len =
        u16::try_from(key_id.len()).map_err(|_| "The key ID is too long.".to_string())?;
    let mut output = Vec::with_capacity(128 + key_id.len());
    output.extend_from_slice(MAGIC);
    output.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
    output.push(KDF_ARGON2ID);
    output.push(CIPHER_XCHACHA20_POLY1305);
    output.extend_from_slice(&memory_kib.to_le_bytes());
    output.extend_from_slice(&iterations.to_le_bytes());
    output.extend_from_slice(&parallelism.to_le_bytes());
    output.extend_from_slice(salt);
    output.extend_from_slice(nonce);
    output.extend_from_slice(public_key);
    output.extend_from_slice(&key_id_len.to_le_bytes());
    output.extend_from_slice(key_id.as_bytes());
    output.extend_from_slice(&(CIPHERTEXT_LEN as u16).to_le_bytes());
    Ok(output)
}

fn parse_container(bytes: &[u8]) -> Result<ParsedContainer<'_>, String> {
    if bytes.len() > MAX_CONTAINER_BYTES {
        return Err("The encrypted USB key container is too large.".into());
    }
    let minimum = MAGIC.len()
        + 2
        + 2
        + 12
        + SALT_LEN
        + NONCE_LEN
        + PUBLIC_KEY_LEN
        + 2
        + 1
        + 2
        + CIPHERTEXT_LEN;
    if bytes.len() < minimum {
        return Err("The encrypted USB key container is truncated.".into());
    }
    let mut cursor = 0_usize;
    if take(bytes, &mut cursor, MAGIC.len())? != MAGIC {
        return Err("The encrypted USB key has an invalid file header.".into());
    }
    let version = read_u16(bytes, &mut cursor)?;
    if version != FORMAT_VERSION {
        return Err("The encrypted USB key format version is not supported.".into());
    }
    if read_u8(bytes, &mut cursor)? != KDF_ARGON2ID
        || read_u8(bytes, &mut cursor)? != CIPHER_XCHACHA20_POLY1305
    {
        return Err("The encrypted USB key uses an unsupported algorithm.".into());
    }
    let memory_kib = read_u32(bytes, &mut cursor)?;
    let iterations = read_u32(bytes, &mut cursor)?;
    let parallelism = read_u32(bytes, &mut cursor)?;
    validate_kdf_params(memory_kib, iterations, parallelism)?;
    let salt = take_array(bytes, &mut cursor)?;
    let nonce = take_array(bytes, &mut cursor)?;
    let public_key = take_array(bytes, &mut cursor)?;
    let key_id_len = usize::from(read_u16(bytes, &mut cursor)?);
    let key_id_bytes = take(bytes, &mut cursor, key_id_len)?;
    let key_id = std::str::from_utf8(key_id_bytes)
        .map_err(|_| "The encrypted USB key ID is not valid UTF-8.".to_string())?
        .to_owned();
    validate_key_id(&key_id)?;
    let ciphertext_len = usize::from(read_u16(bytes, &mut cursor)?);
    if ciphertext_len != CIPHERTEXT_LEN || bytes.len() != cursor + ciphertext_len {
        return Err("The encrypted USB key ciphertext has an invalid size.".into());
    }
    let aad = &bytes[..cursor];
    let ciphertext = &bytes[cursor..];
    Ok(ParsedContainer {
        memory_kib,
        iterations,
        parallelism,
        salt,
        nonce,
        public_key,
        aad,
        ciphertext,
    })
}

fn validate_kdf_params(memory_kib: u32, iterations: u32, parallelism: u32) -> Result<(), String> {
    if memory_kib != ARGON2_MEMORY_KIB
        || iterations != ARGON2_ITERATIONS
        || parallelism != ARGON2_PARALLELISM
    {
        return Err("The encrypted USB key has unsupported Argon2 parameters.".into());
    }
    Ok(())
}

fn derive_key(
    password: &str,
    salt: &[u8; SALT_LEN],
    memory_kib: u32,
    iterations: u32,
    parallelism: u32,
    output: &mut [u8; 32],
) -> Result<(), String> {
    validate_kdf_params(memory_kib, iterations, parallelism)?;
    let params = Params::new(memory_kib, iterations, parallelism, Some(output.len()))
        .map_err(|_| "The USB key uses invalid Argon2 parameters.".to_string())?;
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(password.as_bytes(), salt, output)
        .map_err(|_| "Could not derive the USB key encryption key.".to_string())
}

fn validate_key_id(key_id: &str) -> Result<(), String> {
    if key_id.is_empty()
        || key_id.len() > MAX_KEY_ID_BYTES
        || !key_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(
            "The key ID must contain 1-64 ASCII letters, digits, dots, underscores, or hyphens."
                .into(),
        );
    }
    Ok(())
}

fn metadata(key_id: &str, public_key: &[u8; PUBLIC_KEY_LEN]) -> KeyMetadata {
    KeyMetadata {
        schema_version: 1,
        app_id: APP_ID,
        key_id: key_id.to_owned(),
        key_format: "argon2id-xchacha20poly1305-v1",
        public_key: BASE64_STANDARD.encode(public_key),
        public_key_sha256: sha256_hex(public_key),
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(64);
    for byte in digest {
        let _ = write!(&mut output, "{byte:02x}");
    }
    output
}

fn take<'a>(bytes: &'a [u8], cursor: &mut usize, count: usize) -> Result<&'a [u8], String> {
    let end = cursor
        .checked_add(count)
        .filter(|end| *end <= bytes.len())
        .ok_or_else(|| "The encrypted USB key container is truncated.".to_string())?;
    let output = &bytes[*cursor..end];
    *cursor = end;
    Ok(output)
}

fn take_array<const N: usize>(bytes: &[u8], cursor: &mut usize) -> Result<[u8; N], String> {
    take(bytes, cursor, N)?
        .try_into()
        .map_err(|_| "The encrypted USB key container is truncated.".to_string())
}

fn read_u8(bytes: &[u8], cursor: &mut usize) -> Result<u8, String> {
    Ok(take(bytes, cursor, 1)?[0])
}

fn read_u16(bytes: &[u8], cursor: &mut usize) -> Result<u16, String> {
    Ok(u16::from_le_bytes(take_array(bytes, cursor)?))
}

fn read_u32(bytes: &[u8], cursor: &mut usize) -> Result<u32, String> {
    Ok(u32::from_le_bytes(take_array(bytes, cursor)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_PASSWORD: &str = "correct horse battery staple";

    #[test]
    fn encrypted_container_round_trip_keeps_the_same_seed() {
        let seed = [7_u8; 32];
        let (container, metadata) = encrypt_seed(&seed, TEST_PASSWORD, "test-key").unwrap();
        let decrypted = decrypt_seed(&container, TEST_PASSWORD).unwrap();
        assert_eq!(&*decrypted.seed, &seed);
        assert_eq!(
            decrypted.public_key,
            SigningKey::from_bytes(&seed).verifying_key().to_bytes()
        );
        assert_eq!(
            metadata.public_key,
            BASE64_STANDARD.encode(decrypted.public_key)
        );
    }

    #[test]
    fn wrong_password_and_tampering_fail_closed() {
        let (mut container, _) = encrypt_seed(&[9_u8; 32], TEST_PASSWORD, "test-key").unwrap();
        assert!(decrypt_seed(&container, "this password is definitely wrong").is_err());
        let last = container.len() - 1;
        container[last] ^= 1;
        assert!(decrypt_seed(&container, TEST_PASSWORD).is_err());
    }

    #[test]
    fn trailing_data_and_invalid_key_ids_are_rejected() {
        let (mut container, _) = encrypt_seed(&[3_u8; 32], TEST_PASSWORD, "test-key").unwrap();
        container.push(0);
        assert!(decrypt_seed(&container, TEST_PASSWORD).is_err());
        assert!(encrypt_seed(&[3_u8; 32], TEST_PASSWORD, "bad/key").is_err());
    }
}
