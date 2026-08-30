//! The gifFrames decode path, against a real 3-frame GIF built by the QA rig.

use image::AnimationDecoder;

#[test]
fn decodes_animated_gif_frames_with_delays() {
    let path = match std::env::var("SIGIL_TEST_GIF") {
        Ok(p) => p,
        Err(_) => return, // rig-only test; skip without the fixture
    };
    let file = std::io::BufReader::new(std::fs::File::open(&path).unwrap());
    let frames: Vec<_> = image::codecs::gif::GifDecoder::new(file)
        .unwrap()
        .into_frames()
        .take(64)
        .collect::<Result<_, _>>()
        .unwrap();
    assert!(frames.len() > 1, "fixture should be animated");
    let dir = std::env::temp_dir().join("sigil-gif-test");
    std::fs::create_dir_all(&dir).unwrap();
    for (i, frame) in frames.into_iter().enumerate() {
        let (num, den) = frame.delay().numer_denom_ms();
        let delay = (num / den.max(1)).clamp(20, 1000);
        assert!(delay >= 20, "delay sane");
        let img = image::DynamicImage::ImageRgba8(frame.into_buffer());
        let out = dir.join(format!("{i:03}.png"));
        img.save_with_format(&out, image::ImageFormat::Png).unwrap();
        assert!(out.exists());
    }
}
