use super::*;

fn decode(width: usize, height: usize, bytes: &[u8]) -> Option<String> {
    if !(64..=1280).contains(&width)
        || !(64..=1280).contains(&height)
        || bytes.len() != width * height
    {
        return None;
    }
    let mut image =
        rqrr::PreparedImage::prepare_from_greyscale(width, height, |x, y| bytes[y * width + x]);
    let mut found = None;
    for grid in image.detect_grids().into_iter().take(16) {
        if let Ok((_, text)) = grid.decode() {
            if text.len() <= 4400
                && (text.starts_with("sigil:link:v1:") || text.starts_with("sigil:contact:v1:"))
            {
                if found.is_some() {
                    return None;
                }
                found = Some(text);
            }
        }
    }
    found
}
#[no_mangle]
pub extern "system" fn Java_org_sigil_storage_NativeStorage_scanLinkQr(
    env: JNIEnv,
    _: JObject,
    width: jint,
    height: jint,
    bytes: JByteArray,
) -> jstring {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Option<String> {
        if !(64..=1280).contains(&width)
            || !(64..=1280).contains(&height)
            || env.get_array_length(&bytes).ok()? != width * height
        {
            return None;
        }
        let pixels = Zeroizing::new(env.convert_byte_array(&bytes).ok()?);
        decode(width as usize, height as usize, &pixels)
    }));
    result
        .ok()
        .flatten()
        .and_then(|text| env.new_string(text).ok())
        .map(|text| text.into_raw())
        .unwrap_or(std::ptr::null_mut())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dense_link_frames_decode_after_rotation_and_reject_other_content() {
        for text in [
            format!("sigil:link:v1:proposal:{}", "a7".repeat(800)),
            "https://example.com".into(),
        ] {
            let qr = qrcode::QrCode::new(text.as_bytes()).unwrap();
            let side = (qr.width() + 8) * 5;
            let mut bytes = vec![255; side * side];
            for y in 0..qr.width() {
                for x in 0..qr.width() {
                    if qr[(x, y)] == qrcode::Color::Dark {
                        for dy in 0..5 {
                            for dx in 0..5 {
                                bytes[((y + 4) * 5 + dy) * side + (x + 4) * 5 + dx] = 0;
                            }
                        }
                    }
                }
            }
            let expected = text.starts_with("sigil:").then_some(text);
            assert_eq!(decode(side, side, &bytes), expected);
            let mut rotated = vec![255; bytes.len()];
            for y in 0..side {
                for x in 0..side {
                    rotated[x * side + side - 1 - y] = bytes[y * side + x];
                }
            }
            assert_eq!(decode(side, side, &rotated), expected);
            assert_eq!(decode(side, side, &bytes[..bytes.len() - 1]), None);
        }
        assert_eq!(decode(1281, 1281, &[]), None);
        assert_eq!(decode(64, 64, &[255; 4096]), None);
    }
}
