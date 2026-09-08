#![cfg(all(feature = "worker", target_os = "linux"))]
use sigil_media::{
    formats::Format,
    sandbox::{Input, Runtime},
    Content, Preview, Request,
};
use std::{io::Write, path::Path, sync::atomic::AtomicBool};
fn render(runtime: &Runtime, bytes: &[u8], format: Format, request: &Request) -> Preview {
    let mut input = Input::new().unwrap();
    input.write_all(bytes).unwrap();
    runtime
        .render(&input, format, request, &AtomicBool::new(false))
        .unwrap_or_else(|error| panic!("{format:?} {request:?}: {error}"))
}
fn zip(parts: &[(&str, &str)]) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, content) in parts {
        zip.start_file(*name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(content.as_bytes()).unwrap();
    }
    zip.finish().unwrap().into_inner()
}
fn ffmpeg(output: &Path, args: &[&str]) {
    let status = std::process::Command::new("ffmpeg")
        .args(["-v", "error", "-y", "-threads", "1", "-filter_threads", "1"])
        .args(args)
        .arg(output)
        .status()
        .unwrap();
    assert!(status.success());
}
#[test]
#[ignore = "requires the pinned PDFium and isolated Office runtime; run the attachment acceptance command"]
fn pdf_office_spreadsheet_image_audio_video_and_gif_render_in_isolation() {
    let pdfium = std::env::var_os("SIGIL_TEST_PDFIUM")
        .expect("SIGIL_TEST_PDFIUM")
        .into();
    let office = std::env::var_os("SIGIL_TEST_OFFICE_LIBRARIES")
        .expect("SIGIL_TEST_OFFICE_LIBRARIES")
        .into();
    let runtime = Runtime {
        worker: env!("CARGO_BIN_EXE_sigil-preview").into(),
        pdfium: Some(pdfium),
        office_libraries: Some(office),
        office_language_data: std::env::var_os("SIGIL_TEST_OFFICE_LANGUAGE_DATA").map(Into::into),
    };
    let docx = zip(&[
        (
            "[Content_Types].xml",
            r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#,
        ),
        (
            "_rels/.rels",
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="r1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#,
        ),
        (
            "word/document.xml",
            r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>Synthetic document</w:t></w:r></w:p><w:p><w:r><w:rPr><w:b/></w:rPr><w:t>Bold paragraph</w:t></w:r></w:p><w:sectPr/></w:body></w:document>"#,
        ),
    ]);
    let page = render(
        &runtime,
        &docx,
        Format::Document,
        &Request::Page {
            index: 0,
            width: 640,
        },
    );
    assert!(matches!(
        page.content,
        Content::Image {
            width: 640,
            pages: 1,
            ..
        }
    ));
    assert!(page
        .bytes
        .as_chunks::<4>()
        .0
        .iter()
        .any(|p| p[0] < 100 && p[1] < 100 && p[2] < 100));
    let pptx = zip(&[
        (
            "[Content_Types].xml",
            r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/ppt/presentation.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"/><Override PartName="/ppt/slides/slide1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slide+xml"/></Types>"#,
        ),
        (
            "_rels/.rels",
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="r1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="ppt/presentation.xml"/></Relationships>"#,
        ),
        (
            "ppt/presentation.xml",
            r#"<p:presentation xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><p:sldIdLst><p:sldId id="256" r:id="r1"/></p:sldIdLst><p:sldSz cx="9144000" cy="6858000"/><p:notesSz cx="6858000" cy="9144000"/></p:presentation>"#,
        ),
        (
            "ppt/_rels/presentation.xml.rels",
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="r1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide1.xml"/></Relationships>"#,
        ),
        (
            "ppt/slides/slide1.xml",
            r#"<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/><a:chOff x="0" y="0"/><a:chExt cx="0" cy="0"/></a:xfrm></p:grpSpPr><p:sp><p:nvSpPr><p:cNvPr id="2" name="Synthetic rectangle"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x="914400" y="914400"/><a:ext cx="3657600" cy="1828800"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom><a:solidFill><a:srgbClr val="FF0000"/></a:solidFill></p:spPr><p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:rPr lang="en-US" sz="2400"/><a:t>Synthetic slide</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld></p:sld>"#,
        ),
    ]);
    let slide = render(
        &runtime,
        &pptx,
        Format::Presentation,
        &Request::Page {
            index: 0,
            width: 640,
        },
    );
    assert!(matches!(slide.content, Content::Image { pages: 1, .. }));
    assert!(slide
        .bytes
        .as_chunks::<4>()
        .0
        .iter()
        .any(|p| p[0] > 200 && p[1] < 50 && p[2] < 50));
    let xlsx = zip(&[
        (
            "[Content_Types].xml",
            r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>"#,
        ),
        (
            "_rels/.rels",
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="r1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#,
        ),
        (
            "xl/workbook.xml",
            r#"<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Data" sheetId="1" r:id="r1"/></sheets></workbook>"#,
        ),
        (
            "xl/_rels/workbook.xml.rels",
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="r1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#,
        ),
        (
            "xl/worksheets/sheet1.xml",
            r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="3"><c r="C3" t="inlineStr"><is><t>synthetic</t></is></c><c r="D3"><f>1+2</f><v>3</v></c></row></sheetData></worksheet>"#,
        ),
    ]);
    let table = render(
        &runtime,
        &xlsx,
        Format::Spreadsheet,
        &Request::Table {
            sheet: 0,
            row: 0,
            column: 0,
        },
    );
    let Content::Table {
        cells,
        total_rows,
        total_columns,
        ..
    } = table.content
    else {
        panic!()
    };
    assert_eq!((total_rows, total_columns), (3, 4));
    assert_eq!(cells[2][2], "synthetic");
    assert_eq!(cells[2][3], "3");
    assert!(cells[0][0].is_empty());
    let dir = tempfile::tempdir().unwrap();
    for extension in [
        "png", "jpg", "webp", "bmp", "tiff", "avif", "gif", "mp4", "webm", "mov", "avi", "m4v",
    ] {
        let file = dir.path().join(format!("sample.{extension}"));
        let moving = matches!(extension, "gif" | "mp4" | "webm" | "mov" | "avi" | "m4v");
        if moving {
            ffmpeg(
                &file,
                &[
                    "-f",
                    "lavfi",
                    "-i",
                    "testsrc2=size=96x64:rate=10",
                    "-t",
                    "1",
                ],
            );
        } else {
            ffmpeg(
                &file,
                &[
                    "-f",
                    "lavfi",
                    "-i",
                    "color=c=red:size=96x64",
                    "-frames:v",
                    "1",
                ],
            );
        }
        let format = if extension == "gif" {
            Format::Gif
        } else if moving {
            Format::Video
        } else {
            Format::Image
        };
        let frame = render(
            &runtime,
            &std::fs::read(&file).unwrap(),
            format,
            &Request::Frame {
                at_ms: 0,
                width: 96,
            },
        );
        assert!(
            matches!(
                frame.content,
                Content::Frame {
                    width: 96,
                    height: 64,
                    ..
                }
            ),
            "{extension}"
        );
        if moving {
            let next = render(
                &runtime,
                &std::fs::read(&file).unwrap(),
                format,
                &Request::Frame {
                    at_ms: 500,
                    width: 96,
                },
            );
            assert_ne!(frame.bytes, next.bytes, "{extension}");
        }
    }
    let heic = dir.path().join("sample.heic");
    assert!(std::process::Command::new("heif-enc")
        .arg("-o")
        .arg(&heic)
        .arg(dir.path().join("sample.png"))
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap()
        .success());
    render(
        &runtime,
        &std::fs::read(heic).unwrap(),
        Format::Image,
        &Request::Frame {
            at_ms: 0,
            width: 96,
        },
    );
    for extension in [
        "wav", "mp3", "flac", "ogg", "m4a", "opus", "aac", "aiff", "caf",
    ] {
        let file = dir.path().join(format!("tone.{extension}"));
        ffmpeg(
            &file,
            &[
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:sample_rate=48000",
                "-t",
                "1",
            ],
        );
        let audio = render(
            &runtime,
            &std::fs::read(file).unwrap(),
            Format::Audio,
            &Request::Audio {
                start_ms: 250,
                duration_ms: 100,
            },
        );
        assert!(
            matches!(audio.content,Content::Audio {frames,..} if frames>0),
            "{extension}"
        );
    }
}
