use super::*;
use symphonia::core::{
    formats::{probe::Hint, SeekMode, SeekTo, TrackType},
    io::MediaSourceStream,
    units::Time,
};
pub(super) fn render(path: &Path, start_ms: u64, duration_ms: u32) -> Result<Preview, Error> {
    let source = MediaSourceStream::new(Box::new(std::fs::File::open(path)?), Default::default());
    let mut reader = symphonia::default::get_probe()
        .probe(&Hint::new(), source, Default::default(), Default::default())
        .map_err(|_| Error::Unsupported)?;
    let track = reader
        .default_track(TrackType::Audio)
        .ok_or(Error::Unsupported)?;
    let id = track.id;
    let timebase = track.time_base.ok_or(Error::Invalid)?;
    let parameters = track
        .codec_params
        .as_ref()
        .and_then(|p| p.audio())
        .ok_or(Error::Unsupported)?;
    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(parameters, &Default::default())
        .map_err(|_| Error::Unsupported)?;
    if start_ms > 0 {
        let time = Time::try_new(
            (start_ms / 1000) as i64,
            ((start_ms % 1000) * 1_000_000) as u32,
        )
        .ok_or(Error::Invalid)?;
        reader
            .seek(
                SeekMode::Accurate,
                SeekTo::Time {
                    time,
                    track_id: Some(id),
                },
            )
            .map_err(|_| Error::Unsupported)?;
        decoder.reset();
    }
    let mut rate = 0;
    let mut channels = 0;
    let mut bytes = Vec::new();
    let mut samples = Vec::<f32>::new();
    let mut packets = 0;
    while let Some(packet) = reader.next_packet().map_err(|_| Error::Invalid)? {
        packets += 1;
        if packets > 100_000 {
            return Err(Error::Limit);
        }
        if packet.track_id != id {
            continue;
        }
        let audio = decoder.decode(&packet).map_err(|_| Error::Invalid)?;
        let sample_rate = audio.spec().rate();
        let channel_count = audio.spec().channels().count();
        if !(8000..=192000).contains(&sample_rate) || !(1..=8).contains(&channel_count) {
            return Err(Error::Limit);
        }
        if rate != 0 && (rate != sample_rate || channels != channel_count) {
            return Err(Error::Unsupported);
        }
        rate = sample_rate;
        channels = channel_count;
        if audio.samples_interleaved() > 192000 * 8 {
            return Err(Error::Limit);
        }
        samples.resize(audio.samples_interleaved(), 0.0);
        audio.copy_to_slice_interleaved(&mut samples);
        let time = timebase
            .calc_time(packet.pts)
            .ok_or(Error::Invalid)?
            .as_nanos();
        let start = start_ms as i128 * 1_000_000;
        let skip = ((start - time).max(0) * rate as i128 / 1_000_000_000) as usize;
        let wanted = duration_ms as usize * rate as usize / 1000 * channels * 4;
        for sample in samples.iter().skip(skip.saturating_mul(channels)) {
            if bytes.len() == wanted {
                break;
            }
            if !sample.is_finite() {
                return Err(Error::Invalid);
            }
            bytes.extend_from_slice(&sample.clamp(-1.0, 1.0).to_le_bytes());
        }
        if bytes.len() >= wanted {
            break;
        }
    }
    if rate == 0 || channels == 0 {
        return Err(Error::Invalid);
    }
    Ok(Preview {
        content: Content::Audio {
            rate,
            channels: channels as u8,
            start_ms,
            frames: (bytes.len() / 4 / channels) as u32,
        },
        bytes,
    })
}
