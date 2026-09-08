use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Format {
    Text,
    Csv,
    Tsv,
    Pdf,
    Spreadsheet,
    Document,
    Presentation,
    Stl,
    ThreeMf,
    Contact,
    Image,
    Audio,
    Video,
    Gif,
    Unknown,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fidelity {
    Full,
    Simplified,
    DownloadOnly,
}
pub struct Capability {
    pub format: Format,
    pub extensions: &'static [&'static str],
    pub fidelity: Fidelity,
    pub detail: &'static str,
}
pub const CAPABILITIES: &[Capability] = &[
    Capability {
        format: Format::Text,
        extensions: &[
            "txt", "md", "json", "jsonl", "xml", "yaml", "yml", "toml", "log", "rs", "c", "cpp",
            "h", "html", "css",
        ],
        fidelity: Fidelity::Full,
        detail: "Paged UTF-8 text; markup and scripts are displayed literally",
    },
    Capability {
        format: Format::Csv,
        extensions: &["csv"],
        fidelity: Fidelity::Full,
        detail: "Paged cells; no formula execution",
    },
    Capability {
        format: Format::Tsv,
        extensions: &["tsv"],
        fidelity: Fidelity::Full,
        detail: "Paged cells; no formula execution",
    },
    Capability {
        format: Format::Pdf,
        extensions: &["pdf"],
        fidelity: Fidelity::Full,
        detail: "Rasterized pages; JavaScript and XFA disabled",
    },
    Capability {
        format: Format::Spreadsheet,
        extensions: &["xls", "xlsx", "xlsb", "ods"],
        fidelity: Fidelity::Simplified,
        detail: "Sheet/cell navigation and cached values; no charts, formatting or recalculation",
    },
    Capability {
        format: Format::Document,
        extensions: &["docx", "doc", "odt", "rtf"],
        fidelity: Fidelity::Simplified,
        detail: "Static paginated Office rendering; font/layout differences possible",
    },
    Capability {
        format: Format::Presentation,
        extensions: &["pptx", "ppt", "odp"],
        fidelity: Fidelity::Simplified,
        detail: "Static slides; no transitions or embedded media playback",
    },
    Capability {
        format: Format::Stl,
        extensions: &["stl"],
        fidelity: Fidelity::Simplified,
        detail: "Rotatable solid geometry preview",
    },
    Capability {
        format: Format::ThreeMf,
        extensions: &["3mf"],
        fidelity: Fidelity::Simplified,
        detail: "Rotatable assembled geometry; textures and manufacturing extensions omitted",
    },
    Capability {
        format: Format::Contact,
        extensions: &["vcf", "vcard"],
        fidelity: Fidelity::Simplified,
        detail: "Literal contact properties; no automatic import or remote photo loading",
    },
    Capability {
        format: Format::Image,
        extensions: &[
            "png", "jpg", "jpeg", "webp", "bmp", "tif", "tiff", "avif", "heic", "heif",
        ],
        fidelity: Fidelity::Full,
        detail: "Raster frames; conversion to RGBA may reduce HDR/color fidelity",
    },
    Capability {
        format: Format::Audio,
        extensions: &[
            "wav", "mp3", "flac", "ogg", "oga", "opus", "m4a", "aac", "aiff", "aif", "caf",
        ],
        fidelity: Fidelity::Full,
        detail: "Bounded seekable PCM windows; codec availability is checked by the worker",
    },
    Capability {
        format: Format::Video,
        extensions: &["mp4", "mkv", "webm", "mov", "avi", "m4v"],
        fidelity: Fidelity::Full,
        detail:
            "Seekable video frames and audio windows; codec availability is checked by the worker",
    },
    Capability {
        format: Format::Gif,
        extensions: &["gif"],
        fidelity: Fidelity::Full,
        detail: "Animated timeline frames",
    },
];
pub fn identify(name: &str) -> Format {
    let Some((_, ext)) = name.rsplit_once('.') else {
        return Format::Unknown;
    };
    CAPABILITIES
        .iter()
        .find(|c| c.extensions.iter().any(|e| ext.eq_ignore_ascii_case(e)))
        .map_or(Format::Unknown, |c| c.format)
}
