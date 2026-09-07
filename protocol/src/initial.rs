//! Native initial delivery framing. Both enclosed ciphertexts must authenticate
//! before accepting the message or consuming its one-time prekey.
const PREFIX: &[u8; 8] = b"SGHI\0\x02\0\0";
const FRAME: usize = 12;
const INITIAL_OVERHEAD: usize = 1694;
pub const MAX_PLAINTEXT: usize = 65536 - FRAME - 150;
pub const MAX_BOOTSTRAP: usize = 150 + MAX_PLAINTEXT;

pub fn encode(initial: &[u8], bootstrap: &[u8]) -> Result<Vec<u8>, &'static str> {
    lengths(initial.len(), bootstrap.len())?;
    let mut bytes = Vec::with_capacity(FRAME + initial.len() + bootstrap.len());
    bytes.extend_from_slice(PREFIX);
    bytes.extend_from_slice(&(initial.len() as u32).to_be_bytes());
    bytes.extend_from_slice(initial);
    bytes.extend_from_slice(bootstrap);
    Ok(bytes)
}

pub fn decode(bytes: &[u8]) -> Result<(&[u8], &[u8]), &'static str> {
    if bytes.len() < FRAME
        || bytes.len() > crate::mailbox::MAX_PAYLOAD_HEX / 2
        || &bytes[..8] != PREFIX
    {
        return Err("invalid initial envelope");
    }
    let size = u32::from_be_bytes(bytes[8..12].try_into().map_err(|_| "invalid length")?) as usize;
    let (initial, bootstrap) = bytes[FRAME..]
        .split_at_checked(size)
        .ok_or("invalid length")?;
    lengths(initial.len(), bootstrap.len())?;
    Ok((initial, bootstrap))
}

fn lengths(initial: usize, bootstrap: usize) -> Result<(), &'static str> {
    if initial != INITIAL_OVERHEAD || !(82..=MAX_BOOTSTRAP).contains(&bootstrap) {
        return Err("invalid initial envelope length");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_is_bounded_and_unambiguous() {
        let initial = vec![1; INITIAL_OVERHEAD];
        let bootstrap = vec![2; MAX_BOOTSTRAP];
        let bytes = encode(&initial, &bootstrap).unwrap();
        let mut legacy = bytes.clone();
        legacy[5] = 1;
        assert!(decode(&legacy).is_err());
        assert_eq!(bytes.len() * 2, crate::mailbox::MAX_PAYLOAD_HEX);
        assert_eq!(
            decode(&bytes).unwrap(),
            (initial.as_slice(), bootstrap.as_slice())
        );
        for end in 0..FRAME + INITIAL_OVERHEAD + 82 {
            assert!(decode(&bytes[..end]).is_err());
        }
        for index in 0..FRAME {
            let mut changed = bytes.clone();
            changed[index] ^= 128;
            assert!(decode(&changed).is_err());
        }
        assert!(encode(&initial, &[0; MAX_BOOTSTRAP + 1]).is_err());
        assert!(encode(&vec![0; initial.len() + 1], &bootstrap).is_err());
        assert!(decode(&[bytes.as_slice(), &[0]].concat()).is_err());
    }
}
