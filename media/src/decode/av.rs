use super::*;
use std::process::{Command, Stdio};
pub(super) fn output(command: &mut Command, max: usize) -> Result<Vec<u8>, Error> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| Error::Unavailable)?;
    let mut bytes = Vec::new();
    let result = child
        .stdout
        .take()
        .ok_or(Error::Io)?
        .take(max as u64 + 1)
        .read_to_end(&mut bytes);
    if result.is_err() || bytes.len() > max {
        let _ = child.kill();
        let _ = child.wait();
        return Err(Error::Limit);
    }
    if !child.wait()?.success() {
        return Err(Error::Invalid);
    }
    Ok(bytes)
}
fn ffmpeg(path: &Path, at: u64) -> Command {
    let mut cmd = Command::new("/usr/bin/ffmpeg");
    cmd.args([
        "-nostdin",
        "-v",
        "error",
        "-threads",
        "1",
        "-filter_threads",
        "1",
        "-protocol_whitelist",
        "file,pipe",
        "-ss",
        &format!("{}.{:03}", at / 1000, at % 1000),
        "-i",
    ])
    .arg(path);
    cmd
}
pub(super) fn frame(path: &Path, at_ms: u64, width: u32) -> Result<Preview, Error> {
    let mut probe = Command::new("/usr/bin/ffprobe");
    probe
        .args([
            "-v",
            "error",
            "-protocol_whitelist",
            "file,pipe",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=width,height:stream_side_data=rotation:format=duration",
            "-of",
            "json",
        ])
        .arg(path);
    let data: serde_json::Value =
        serde_json::from_slice(&output(&mut probe, 65536)?).map_err(|_| Error::Invalid)?;
    let stream = data["streams"]
        .as_array()
        .and_then(|a| a.first())
        .ok_or(Error::Unsupported)?;
    let mut original_width = stream["width"].as_u64().ok_or(Error::Invalid)?;
    let mut original_height = stream["height"].as_u64().ok_or(Error::Invalid)?;
    if let Some(rotation) = stream["side_data_list"]
        .as_array()
        .and_then(|a| a.iter().find_map(|v| v["rotation"].as_i64()))
    {
        if rotation.rem_euclid(180) == 90 {
            std::mem::swap(&mut original_width, &mut original_height);
        }
    }
    if original_width == 0
        || original_height == 0
        || original_width > 32768
        || original_height > 32768
        || original_width * original_height > 100_000_000
    {
        return Err(Error::Limit);
    }
    let scale = (width as f64 / original_width as f64)
        .min(2048.0 / original_height as f64)
        .min(1.0);
    let w = ((original_width as f64 * scale).round() as u32).max(1);
    let h = ((original_height as f64 * scale).round() as u32).max(1);
    let duration_ms = data["format"]["duration"]
        .as_str()
        .and_then(|v| v.parse::<f64>().ok())
        .filter(|v| v.is_finite() && *v >= 0.0 && *v <= 7.0 * 86400.0)
        .map(|v| (v * 1000.0).round() as u64);
    let size = crate::pixels(w, h)?;
    let mut cmd = ffmpeg(path, at_ms);
    cmd.args([
        "-map",
        "0:v:0",
        "-frames:v",
        "1",
        "-vf",
        &format!("scale={w}:{h}"),
        "-pix_fmt",
        "rgba",
        "-f",
        "rawvideo",
        "pipe:1",
    ]);
    let bytes = output(&mut cmd, size)?;
    Ok(Preview {
        content: Content::Frame {
            width: w,
            height: h,
            at_ms,
            duration_ms,
        },
        bytes,
    })
}
pub(super) fn audio(path: &Path, start_ms: u64, duration_ms: u32) -> Result<Preview, Error> {
    let mut cmd = ffmpeg(path, start_ms);
    cmd.args([
        "-map",
        "0:a:0",
        "-vn",
        "-t",
        &format!("{}.{:03}", duration_ms / 1000, duration_ms % 1000),
        "-ar",
        "48000",
        "-ac",
        "2",
        "-f",
        "f32le",
        "pipe:1",
    ]);
    let bytes = output(&mut cmd, duration_ms as usize * 48 * 2 * 4)?;
    if bytes.len() % 8 != 0 {
        return Err(Error::Invalid);
    }
    Ok(Preview {
        content: Content::Audio {
            rate: 48000,
            channels: 2,
            start_ms,
            frames: (bytes.len() / 8) as u32,
        },
        bytes,
    })
}
