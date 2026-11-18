use aes_gcm::{aead::{Aead, KeyInit}, Aes256Gcm, Nonce};
use anyhow::{anyhow, bail, Result};
use rand::RngCore;
use sha2::{Digest, Sha256};

const MAGIC: &[u8; 8] = b"LADONv1!";

pub fn digest(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

pub fn is_digest(hash: &str) -> bool {
    hash.len() == 64 && hash.bytes().all(|c| c.is_ascii_hexdigit())
}

pub fn load_archive_key() -> Result<[u8; 32]> {
    let value = std::env::var("LADON_ARCHIVE_KEY_HEX")
        .map_err(|_| anyhow!("LADON_ARCHIVE_KEY_HEX required to upload/decrypt archive"))?;
    let raw = hex::decode(value.trim()).map_err(|_| anyhow!("archive key must be 64 hex digits"))?;
    raw.try_into().map_err(|_| anyhow!("archive key must be exactly 32 bytes"))
}

pub fn seal(data: &[u8], key: &[u8; 32]) -> Result<Vec<u8>> {
    let compressed = zstd::stream::encode_all(data, 3)?;
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|_| anyhow!("invalid AES key"))?;
    let mut nonce = [0u8; 12];
    rand::rngs::OsRng.fill_bytes(&mut nonce);
    let encrypted = cipher.encrypt(Nonce::from_slice(&nonce), compressed.as_ref())
        .map_err(|_| anyhow!("encryption failed"))?;
    let mut output = Vec::with_capacity(MAGIC.len() + 12 + encrypted.len());
    output.extend_from_slice(MAGIC);
    output.extend_from_slice(&nonce);
    output.extend_from_slice(&encrypted);
    Ok(output)
}

pub fn open(payload: &[u8], key: &[u8; 32], expected_hash: &str) -> Result<Vec<u8>> {
    if payload.len() < 36 || !payload.starts_with(MAGIC) { bail!("invalid archive envelope"); }
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|_| anyhow!("invalid AES key"))?;
    let plain = cipher.decrypt(Nonce::from_slice(&payload[8..20]), &payload[20..])
        .map_err(|_| anyhow!("archive authentication failed"))?;
    let decompressed = zstd::stream::decode_all(plain.as_slice())?;
    if digest(&decompressed) != expected_hash { bail!("sha256 mismatch after archive decrypt"); }
    Ok(decompressed)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ut_crypto_01_roundtrip() {
        let key=[2u8;32]; let input=b"sensitive content 123";
        let sealed=seal(input,&key).unwrap();
        assert_ne!(sealed,input);
        assert_eq!(open(&sealed,&key,&digest(input)).unwrap(),input);
    }
    #[test]
    fn ut_crypto_02_tamper_is_rejected() {
        let key=[2u8;32]; let mut sealed=seal(b"secret",&key).unwrap();
        let i=sealed.len()-1; sealed[i]^=1;
        assert!(open(&sealed,&key,&digest(b"secret")).is_err());
    }
    #[test]
    fn ut_crypto_03_digest_validation() {
        assert!(is_digest(&digest(b"abc")));
        assert!(!is_digest("../../windows"));
    }
}
