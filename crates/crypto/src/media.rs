//! Bounded whole-file XChaCha20-Poly1305 attachments, separate from MLS and vault limits.
use crate::*;
use croaker_protocol::{MessageId, media::*};
pub struct PreparedObject {
    pub descriptor: ObjectAttachment,
    pub ciphertext: Vec<u8>,
}
fn aad(d: &ObjectAttachment) -> Vec<u8> {
    let mut a = MEDIA_HEADER.to_vec();
    a.extend_from_slice(d.conversation_id.0.as_bytes());
    a.extend_from_slice(d.object_id.0.as_bytes());
    a.extend_from_slice(&d.size.to_be_bytes());
    for s in [&d.name, &d.mime] {
        a.extend_from_slice(&(s.len() as u32).to_be_bytes());
        a.extend_from_slice(s.as_bytes());
    }
    a
}
pub fn encrypt(
    conversation_id: ConversationId,
    name: &str,
    mime: &str,
    plaintext: &[u8],
) -> Result<PreparedObject> {
    if plaintext.len() > MAX_MEDIA_BYTES {
        return Err(CryptoError::InvalidInput);
    }
    let mut key = Zeroizing::new([0u8; 32]);
    OsRng.fill_bytes(key.as_mut());
    let mut nonce = [0u8; 24];
    OsRng.fill_bytes(&mut nonce);
    let mut descriptor = ObjectAttachment {
        version: 1,
        conversation_id,
        object_id: MessageId::new(),
        name: name.into(),
        mime: mime.into(),
        size: plaintext.len() as u64,
        ciphertext_size: (plaintext.len() + MEDIA_OVERHEAD) as u64,
        key: BASE64_STANDARD.encode(key.as_ref()),
        sha256: "0".repeat(64),
    };
    descriptor
        .validate()
        .map_err(|_| CryptoError::InvalidInput)?;
    let sealed = XChaCha20Poly1305::new((&*key).into())
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: plaintext,
                aad: &aad(&descriptor),
            },
        )
        .map_err(|_| CryptoError::InvalidInput)?;
    let mut ciphertext = MEDIA_HEADER.to_vec();
    ciphertext.extend_from_slice(&nonce);
    ciphertext.extend_from_slice(&sealed);
    descriptor.sha256 = hex::encode(Sha256::digest(&ciphertext));
    Ok(PreparedObject {
        descriptor,
        ciphertext,
    })
}
pub fn decrypt(descriptor: &ObjectAttachment, ciphertext: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
    descriptor
        .validate()
        .map_err(|_| CryptoError::InvalidInput)?;
    if ciphertext.len() as u64 != descriptor.ciphertext_size
        || !ciphertext.starts_with(MEDIA_HEADER)
        || hex::encode(Sha256::digest(ciphertext)) != descriptor.sha256
    {
        return Err(CryptoError::InvalidInput);
    }
    let key = Zeroizing::new(
        BASE64_STANDARD
            .decode(&descriptor.key)
            .map_err(|_| CryptoError::InvalidInput)?,
    );
    let offset = MEDIA_HEADER.len();
    let plain = XChaCha20Poly1305::new_from_slice(&key)
        .map_err(|_| CryptoError::InvalidInput)?
        .decrypt(
            XNonce::from_slice(&ciphertext[offset..offset + 24]),
            Payload {
                msg: &ciphertext[offset + 24..],
                aad: &aad(descriptor),
            },
        )
        .map_err(|_| CryptoError::InvalidInput)?;
    if plain.len() as u64 != descriptor.size {
        return Err(CryptoError::InvalidInput);
    }
    Ok(Zeroizing::new(plain))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn authenticated_and_randomized() {
        let c = ConversationId::new();
        let p = encrypt(c, "photo.png", "image/png", b"secret").unwrap();
        assert_eq!(&**decrypt(&p.descriptor, &p.ciphertext).unwrap(), b"secret");
        let q = encrypt(c, "photo.png", "image/png", b"secret").unwrap();
        assert_ne!(p.ciphertext, q.ciphertext);
        let mut d = p.descriptor.clone();
        d.name = "other.png".into();
        assert!(decrypt(&d, &p.ciphertext).is_err());
        d = p.descriptor.clone();
        d.conversation_id = ConversationId::new();
        assert!(decrypt(&d, &p.ciphertext).is_err());
        let mut bad = p.ciphertext.clone();
        bad[20] ^= 1;
        assert!(decrypt(&p.descriptor, &bad).is_err());
        assert!(decrypt(&p.descriptor, &p.ciphertext[..20]).is_err());
        d = p.descriptor.clone();
        d.key = BASE64_STANDARD.encode([0u8; 32]);
        assert!(decrypt(&d, &p.ciphertext).is_err());
        d = p.descriptor.clone();
        d.sha256 = hex::encode(Sha256::digest(&bad));
        assert!(decrypt(&d, &bad).is_err());
        d = p.descriptor.clone();
        d.object_id = MessageId::new();
        assert!(decrypt(&d, &p.ciphertext).is_err());
    }
    #[test]
    fn full_media_limit_does_not_expand_vault_limit() {
        let bytes = Zeroizing::new(vec![7; MAX_MEDIA_BYTES]);
        assert!(seal_record(&bytes, &[9; 32], b"test").is_err());
        let p = encrypt(
            ConversationId::new(),
            "large.bin",
            "application/octet-stream",
            &bytes,
        )
        .unwrap();
        assert_eq!(
            decrypt(&p.descriptor, &p.ciphertext).unwrap().as_slice(),
            bytes.as_slice()
        );
    }
    #[test]
    fn bounded() {
        assert!(encrypt(ConversationId::new(), "../x", "", b"").is_err());
        assert!(
            encrypt(
                ConversationId::new(),
                "x",
                "",
                &vec![0; MAX_MEDIA_BYTES + 1]
            )
            .is_err()
        );
        let p = encrypt(ConversationId::new(), "x", "", &vec![7; 33 * 1024]).unwrap();
        assert_eq!(
            decrypt(&p.descriptor, &p.ciphertext).unwrap().len(),
            33 * 1024
        );
    }
}
