#![forbid(unsafe_code)]
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
#[cfg(feature = "worker")]
pub mod decode;
pub mod formats;
#[cfg(target_os = "linux")]
pub mod sandbox;

pub const MAX_INPUT: u64 = 128 * 1024 * 1024;
pub const MAX_OUTPUT: usize = 16 * 1024 * 1024;
pub const MAX_METADATA: usize = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    Invalid,
    Limit,
    Unsupported,
    Unavailable,
    Cancelled,
    Io,
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "preview {self:?}")
    }
}
impl std::error::Error for Error {}
impl Error {
    pub fn exit_code(self) -> i32 {
        match self {
            Self::Invalid => 2,
            Self::Limit => 3,
            Self::Unsupported => 4,
            Self::Unavailable => 5,
            Self::Cancelled => 6,
            Self::Io => 7,
        }
    }
    pub fn from_exit_code(code: Option<i32>) -> Self {
        match code {
            Some(3) => Self::Limit,
            Some(4) => Self::Unsupported,
            Some(5) => Self::Unavailable,
            Some(6) => Self::Cancelled,
            Some(7) => Self::Io,
            _ => Self::Invalid,
        }
    }
}
impl From<std::io::Error> for Error {
    fn from(_: std::io::Error) -> Self {
        Self::Io
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "view", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    Text { offset: u64 },
    Page { index: u32, width: u32 },
    Table { sheet: u32, row: u32, column: u32 },
    Audio { start_ms: u64, duration_ms: u32 },
    Frame { at_ms: u64, width: u32 },
    Mesh { yaw: f32, pitch: f32, width: u32 },
}
impl Request {
    pub fn validate(&self) -> Result<(), Error> {
        match *self {
            Self::Text { offset } if offset <= MAX_INPUT => (),
            Self::Table { sheet, row, column }
                if sheet < 1024 && row < 1_048_576 && column < 16_384 => {}
            Self::Page { index, width } if index < 100_000 && (32..=2048).contains(&width) => (),
            Self::Frame { at_ms, width }
                if at_ms <= 7 * 86_400_000 && (32..=2048).contains(&width) => {}
            Self::Audio {
                start_ms,
                duration_ms,
            } if start_ms <= 7 * 86_400_000 && (1..=10_000).contains(&duration_ms) => (),
            Self::Mesh { yaw, pitch, width }
                if yaw.is_finite()
                    && pitch.is_finite()
                    && yaw.abs() <= 360.0
                    && pitch.abs() <= 360.0
                    && (32..=2048).contains(&width) => {}
            _ => return Err(Error::Limit),
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Content {
    Text {
        text: String,
        next: Option<u64>,
    },
    Table {
        sheets: Vec<String>,
        sheet: u32,
        row: u32,
        column: u32,
        total_rows: u32,
        total_columns: u32,
        cells: Vec<Vec<String>>,
    },
    Image {
        width: u32,
        height: u32,
        pages: u32,
        index: u32,
    },
    Audio {
        rate: u32,
        channels: u8,
        start_ms: u64,
        frames: u32,
    },
    Frame {
        width: u32,
        height: u32,
        at_ms: u64,
        duration_ms: Option<u64>,
    },
    Mesh {
        width: u32,
        height: u32,
        triangles: u32,
    },
}
#[derive(Debug, PartialEq)]
pub struct Preview {
    pub content: Content,
    pub bytes: Vec<u8>,
}
impl Preview {
    pub fn validate(&self) -> Result<(), Error> {
        if self.bytes.len() > MAX_OUTPUT {
            return Err(Error::Limit);
        }
        let expected = match &self.content {
            Content::Text { text, next } => {
                if text.len() > 64 * 1024 || next.is_some_and(|n| n > MAX_INPUT) {
                    return Err(Error::Limit);
                }
                0
            }
            Content::Table {
                sheets,
                sheet,
                row,
                column,
                total_rows,
                total_columns,
                cells,
            } => {
                if sheets.is_empty()
                    || sheets.len() > 1024
                    || *sheet as usize >= sheets.len()
                    || sheets.iter().any(|s| s.len() > 512)
                    || cells.len() > 128
                    || *total_rows > 1_048_576
                    || *total_columns > 16_384
                    || *row > *total_rows
                    || *column > *total_columns
                    || cells.len() as u32 > total_rows.saturating_sub(*row)
                    || cells.iter().any(|r| {
                        r.len() > 32
                            || r.len() as u32 > total_columns.saturating_sub(*column)
                            || r.iter().any(|s| s.len() > 4096)
                    })
                {
                    return Err(Error::Limit);
                }
                0
            }
            Content::Image {
                width,
                height,
                pages,
                index,
            } => {
                if *pages == 0 || *pages > 100_000 || index >= pages {
                    return Err(Error::Invalid);
                }
                pixels(*width, *height)?
            }
            Content::Frame {
                width,
                height,
                at_ms,
                duration_ms,
            } => {
                if *at_ms > 7 * 86_400_000 || duration_ms.is_some_and(|v| v > 7 * 86_400_000) {
                    return Err(Error::Limit);
                }
                pixels(*width, *height)?
            }
            Content::Mesh {
                width,
                height,
                triangles,
            } => {
                if *triangles == 0 || *triangles > 200_000 {
                    return Err(Error::Limit);
                }
                pixels(*width, *height)?
            }
            Content::Audio {
                rate,
                channels,
                start_ms,
                frames,
            } => {
                if !(8000..=192000).contains(rate)
                    || !(1..=8).contains(channels)
                    || *start_ms > 7 * 86_400_000
                    || *frames > rate * 10
                {
                    return Err(Error::Limit);
                }
                if self
                    .bytes
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .any(|bytes| !f32::from_le_bytes(*bytes).is_finite())
                {
                    return Err(Error::Invalid);
                }
                *frames as usize * *channels as usize * 4
            }
        };
        if self.bytes.len() != expected {
            return Err(Error::Invalid);
        }
        Ok(())
    }
    pub fn write(&self, mut writer: impl Write) -> Result<(), Error> {
        self.validate()?;
        let metadata = serde_json::to_vec(&self.content).map_err(|_| Error::Invalid)?;
        if metadata.len() > MAX_METADATA {
            return Err(Error::Limit);
        }
        writer.write_all(b"SGPV\x01")?;
        writer.write_all(&(metadata.len() as u32).to_be_bytes())?;
        writer.write_all(&(self.bytes.len() as u32).to_be_bytes())?;
        writer.write_all(&metadata)?;
        writer.write_all(&self.bytes)?;
        Ok(())
    }
    pub fn read(mut reader: impl Read) -> Result<Self, Error> {
        let mut head = [0u8; 13];
        reader.read_exact(&mut head)?;
        if &head[..5] != b"SGPV\x01" {
            return Err(Error::Invalid);
        }
        let meta = u32::from_be_bytes(head[5..9].try_into().unwrap()) as usize;
        let len = u32::from_be_bytes(head[9..13].try_into().unwrap()) as usize;
        if meta > MAX_METADATA || len > MAX_OUTPUT {
            return Err(Error::Limit);
        }
        let mut metadata = vec![0; meta];
        reader.read_exact(&mut metadata)?;
        let mut bytes = vec![0; len];
        reader.read_exact(&mut bytes)?;
        if reader.read(&mut [0])? != 0 {
            return Err(Error::Invalid);
        }
        let preview = Self {
            content: serde_json::from_slice(&metadata).map_err(|_| Error::Invalid)?,
            bytes,
        };
        preview.validate()?;
        Ok(preview)
    }
}
fn pixels(width: u32, height: u32) -> Result<usize, Error> {
    if width == 0 || height == 0 || width > 2048 || height > 2048 {
        return Err(Error::Limit);
    }
    Ok(width as usize * height as usize * 4)
}
