use base64ct::{Base64UrlUnpadded, Encoding};
use once_cell::sync::Lazy;

use super::*;

macro_rules! DECODE {
    ($e:expr) => {
        Lazy::new(|| {
            let decoded = Base64UrlUnpadded::decode_vec($e).unwrap();
            decoded.try_into().unwrap()
        })
    };
}

mod rfc8188_example1 {
    use super::*;

    pub(crate) const PLAINTEXT: &[u8] = b"I am the walrus";
    const RS: u32 = 4096;
    static IKM: Lazy<[u8; 16]> = DECODE!("yqdlZ-tYemfogSmv7Ws5PQ");
    const KEYID: &[u8] = "".as_bytes();

    static ENCRYPTED: Lazy<[u8; 53]> =
        DECODE!("I1BsxtFttlv3u_Oo94xnmwAAEAAA-NAVub2qFgBEuQKRapoZu-IxkIva3MEB1PD-ly8Thjg");

    static SALT: Lazy<[u8; 16]> = DECODE!("I1BsxtFttlv3u_Oo94xnmw");
    static PRK: Lazy<[u8; 32]> = DECODE!("zyeH5phsIsgUyd4oiSEIy35x-gIi4aM7y0hCF8mwn9g");
    static CEK: Lazy<[u8; 16]> = DECODE!("_wniytB-ofscZDh4tbSjHw");
    static NONCE: Lazy<[u8; 12]> = DECODE!("Bcs8gkIRKLI8GeI8");

    #[test]
    fn test_prk_generation() {
        let (prk, _) = Hkdf::<Sha256>::extract(Some(&*SALT), &*IKM);
        assert_eq!(prk.as_slice().len(), PRK.len());
        assert_eq!(prk.as_slice(), *PRK);
    }

    #[test]
    fn test_key_derivation() {
        assert_eq!(
            &derive_key(*SALT, *IKM),
            aes_gcm::Key::<Aes128Gcm>::from_slice(&*CEK)
        );
    }

    #[test]
    fn test_nonce_derivation() {
        let seq = [0u8; 12];
        assert_eq!(derive_nonce(*SALT, *IKM, seq), Nonce::from(*NONCE));
    }

    #[test]
    fn test_header_generation() {
        let header = generate_encryption_header(*SALT, 18, "").unwrap();
        assert_eq!(header.len(), 21)
    }

    #[test]
    fn test_encryption() {
        let encrypted =
            encrypt(*IKM, *SALT, KEYID, Some(PLAINTEXT.to_vec()).into_iter(), RS).unwrap();

        assert_eq!(encrypted.len(), ENCRYPTED.len());
        assert_eq!(encrypted[..16], ENCRYPTED[..16]);
        assert_eq!(
            u32::from_be_bytes(ENCRYPTED[16..16 + 4].try_into().unwrap()),
            u32::from_be_bytes(encrypted[16..16 + 4].try_into().unwrap())
        );
        assert_eq!(encrypted[21..], ENCRYPTED[21..]);
        assert_eq!(encrypted, &ENCRYPTED[..]);
    }

    #[test]
    fn test_encryption_decryption() {
        let encrypted =
            encrypt(*IKM, *SALT, KEYID, Some(PLAINTEXT.to_vec()).into_iter(), RS).unwrap();
        let decrypted = decrypt(*IKM, encrypted).unwrap();

        assert_eq!(decrypted, PLAINTEXT.to_vec());
    }
}

#[test]
fn malformed_sizes_and_missing_records_return_errors() {
    for rs in 0..18u32 {
        let mut bytes = vec![0u8; 21];
        bytes[16..20].copy_from_slice(&rs.to_be_bytes());
        assert!(decrypt([0; 16], bytes).is_err());
        assert!(generate_encryption_header([0; 16], rs, "").is_err());
    }
    let header = generate_encryption_header([0; 16], 4096, "").unwrap();
    assert!(decrypt([0; 16], header).is_err());
}

#[test]
fn authenticated_padding_requires_exact_record_delimiters() {
    let key = aes_gcm::Key::<Aes128Gcm>::from([7; 16]);
    let nonce = Nonce::from([8; 12]);
    for last in [false, true] {
        for delimiter in 0..=255u8 {
            let mut bytes = vec![b'x', delimiter, 0, 0];
            Aes128Gcm::new(&key)
                .encrypt_in_place(&nonce, b"", &mut bytes)
                .unwrap();
            let result = decrypt_record(&key, &nonce, &mut bytes, last);
            if delimiter == if last { 2 } else { 1 } {
                assert_eq!(result.unwrap(), b"x");
            } else {
                assert!(result.is_err());
            }
        }
    }
}

mod rfc8188_example2 {
    use super::{rfc8188_example1::PLAINTEXT, *};

    const RS: u32 = 25;
    static IKM: Lazy<[u8; 16]> = DECODE!("BO3ZVPxUlnLORbVGMpbT1Q");
    const KEYID: &[u8] = "a1".as_bytes();

    static SALT: Lazy<[u8; 16]> = Lazy::new(|| ENCRYPTED[0..16].try_into().unwrap());
    static ENCRYPTED: Lazy<[u8; 73]> = DECODE!("uNCkWiNYzKTnBN9ji3-qWAAAABkCYTHOG8chz_gnvgOqdGYovxyjuqRyJFjEDyoF1Fvkj6hQPdPHI51OEUKEpgz3SsLWIqS_uA");

    #[test]
    fn test_encryption() {
        let encrypted = encrypt(
            *IKM,
            *SALT,
            KEYID,
            vec![PLAINTEXT[..7].to_vec(), PLAINTEXT[7..7 + 8].to_vec()].into_iter(),
            RS,
        )
        .unwrap();

        assert_eq!(encrypted.len(), ENCRYPTED.len());
        assert_eq!(encrypted[..16], ENCRYPTED[..16]);
        assert_eq!(
            u32::from_be_bytes(ENCRYPTED[16..16 + 4].try_into().unwrap()),
            u32::from_be_bytes(encrypted[16..16 + 4].try_into().unwrap())
        );
        assert_eq!(encrypted[21..], ENCRYPTED[21..]);
        assert_eq!(encrypted, &ENCRYPTED[..]);
    }

    #[test]
    fn test_encryption_decryption() {
        let encrypted = encrypt(
            *IKM,
            *SALT,
            KEYID,
            vec![PLAINTEXT[..7].to_vec(), PLAINTEXT[7..7 + 8].to_vec()].into_iter(),
            RS,
        )
        .unwrap();
        let decrypted = decrypt(*IKM, encrypted).unwrap();

        assert_eq!(decrypted, PLAINTEXT.to_vec());
    }
}
