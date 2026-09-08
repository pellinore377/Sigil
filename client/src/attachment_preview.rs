use super::*;
use sigil_media::{
    formats::{identify, Format},
    sandbox::{Input, Job},
    Preview,
};
use std::{
    io::Write,
    sync::atomic::{AtomicBool, Ordering},
};
pub(super) fn request(bytes: &[u8]) -> Result<(Format, u32), Error> {
    let file = sigil_protocol::file::File::from_bytes(bytes).map_err(|_| Error::InvalidStore)?;
    let descriptor = sigil_protocol::file::KeyDescriptor::from_bytes(file.descriptor)
        .map_err(|_| Error::InvalidStore)?;
    if descriptor.length > sigil_media::MAX_INPUT {
        return Err(sigil_media::Error::Limit.into());
    }
    let format = identify(file.name);
    if format == Format::Unknown {
        return Err(sigil_media::Error::Unsupported.into());
    }
    let count = Shape {
        file: descriptor.file,
        length: descriptor.length,
    }
    .chunks()?;
    Ok((format, count))
}
pub(super) fn collect(
    count: u32,
    cancel: &AtomicBool,
    mut chunk: impl FnMut(u32) -> Result<Zeroizing<Vec<u8>>, Error>,
) -> Result<Input, Error> {
    let mut input = Input::new()?;
    for index in 0..count {
        if cancel.load(Ordering::Acquire) {
            return Err(Error::Cancelled);
        }
        input.write_all(&chunk(index)?)?;
    }
    Ok(input)
}
impl ClientStore {
    /// Authenticate all chunks before worker startup; recheck current visibility before handoff.
    pub fn preview_recovered_file(
        &self,
        cache: &Cache,
        record: Id,
        job: &Job<'_>,
        mut clock: impl FnMut() -> u64,
    ) -> Result<Preview, Error> {
        events::source(self, cache)?;
        let bytes = crate::recovery::recovery_file(&self.db, &self.key, cache.scope, record)?;
        let (format, count) = request(&bytes)?;
        let input = collect(count, job.cancel, |index| {
            self.recovered_file_chunk_for(cache, record, (index, Some(&bytes)), clock())
        })?;
        let preview = job.render(&input, format)?;
        self.recovered_file_chunk_for(cache, record, (0, Some(&bytes)), clock())?;
        Ok(preview)
    }
    pub fn preview_conversation_file(
        &mut self,
        cache: &Cache,
        conversation: Id,
        reference: crate::conversations::Reference,
        job: &Job<'_>,
        mut clock: impl FnMut() -> u64,
    ) -> Result<Preview, Error> {
        events::source(self, cache)?;
        let message = self.conversation_message(conversation, reference.clone(), clock())?;
        if message.view_once {
            return Err(Error::Unprepared);
        }
        let Some(crate::conversations::Body::File(bytes)) = message.body else {
            return Err(Error::InvalidEvent);
        };
        let (format, count) = request(&bytes)?;
        let input = collect(count, job.cancel, |index| {
            self.conversation_file_chunk_for(
                cache,
                conversation,
                reference.clone(),
                (index, Some(&bytes)),
                clock(),
            )
        })?;
        let preview = job.render(&input, format)?;
        self.conversation_file_chunk_for(
            cache,
            conversation,
            reference,
            (0, Some(&bytes)),
            clock(),
        )?;
        Ok(preview)
    }
}
