#![cfg(feature = "worker")]
use sigil_media::{
    decode,
    formats::{self, Format},
    Content, Error, Preview, Request,
};
use std::io::{Cursor, Write};
fn source(bytes: &[u8]) -> tempfile::NamedTempFile {
    let mut file = tempfile::NamedTempFile::new().unwrap();
    file.write_all(bytes).unwrap();
    file
}
fn pcm() -> Vec<u8> {
    let frames = 16000u32;
    let size = frames * 2;
    let mut bytes = b"RIFF".to_vec();
    bytes.extend((36 + size).to_le_bytes());
    bytes.extend(b"WAVEfmt ");
    bytes.extend(16u32.to_le_bytes());
    bytes.extend(1u16.to_le_bytes());
    bytes.extend(1u16.to_le_bytes());
    bytes.extend(8000u32.to_le_bytes());
    bytes.extend(16000u32.to_le_bytes());
    bytes.extend(2u16.to_le_bytes());
    bytes.extend(16u16.to_le_bytes());
    bytes.extend(b"data");
    bytes.extend(size.to_le_bytes());
    for i in 0..frames {
        bytes.extend(((i % 800) as i16 * 30).to_le_bytes());
    }
    bytes
}
#[test]
fn text_and_cells_are_paged_without_executing_or_losing_unicode() {
    let text = format!("{}é<script>literal</script>", "x".repeat(65535));
    let file = source(text.as_bytes());
    let first = decode::render(file.path(), Format::Text, &Request::Text { offset: 0 }).unwrap();
    let Content::Text {
        text: part,
        next: Some(next),
    } = first.content
    else {
        panic!()
    };
    assert_eq!(part.len(), 65535);
    let second =
        decode::render(file.path(), Format::Text, &Request::Text { offset: next }).unwrap();
    assert!(matches!(second.content,Content::Text {text,..} if text=="é<script>literal</script>"));
    let file = source(b"a,b\n1,\"line1\nline2\"\n=HYPERLINK(1),<script>\n");
    let result = decode::render(
        file.path(),
        Format::Csv,
        &Request::Table {
            sheet: 0,
            row: 1,
            column: 1,
        },
    )
    .unwrap();
    let Content::Table {
        cells,
        total_rows,
        total_columns,
        ..
    } = result.content
    else {
        panic!()
    };
    assert_eq!((total_rows, total_columns), (3, 2));
    assert_eq!(cells, vec![vec!["line1\nline2"], vec!["<script>"]]);
    assert_eq!(formats::identify("SCRIPT.HTML"), Format::Text);
    assert_eq!(formats::identify("unknown.exe"), Format::Unknown);
}
#[test]
fn audio_windows_seek_to_known_samples_and_survive_output_roundtrip() {
    let file = source(&pcm());
    let preview = decode::render(
        file.path(),
        Format::Audio,
        &Request::Audio {
            start_ms: 125,
            duration_ms: 100,
        },
    )
    .unwrap();
    assert!(matches!(
        preview.content,
        Content::Audio {
            rate: 8000,
            channels: 1,
            frames: 800,
            ..
        }
    ));
    let sample = f32::from_le_bytes(preview.bytes[..4].try_into().unwrap());
    assert!((sample - 6000.0 / 32768.0).abs() < 0.00001, "{sample}");
    let mut bytes = Vec::new();
    preview.write(&mut bytes).unwrap();
    assert_eq!(Preview::read(bytes.as_slice()).unwrap(), preview);
    bytes.push(0);
    assert!(Preview::read(bytes.as_slice()).is_err());
}
#[test]
fn geometry_renders_and_assembly_transforms_change_the_silhouette() {
    let stl = b"solid test\nfacet normal 0 0 1\nouter loop\nvertex 0 0 0\nvertex 1 0 0\nvertex 0 1 0\nendloop\nendfacet\nendsolid test\n";
    let file = source(stl);
    let first = decode::render(
        file.path(),
        Format::Stl,
        &Request::Mesh {
            yaw: 0.,
            pitch: 0.,
            width: 128,
        },
    )
    .unwrap();
    let second = decode::render(
        file.path(),
        Format::Stl,
        &Request::Mesh {
            yaw: 45.,
            pitch: 30.,
            width: 128,
        },
    )
    .unwrap();
    assert_ne!(first.bytes, second.bytes);
    assert!(first
        .bytes
        .as_chunks::<4>()
        .0
        .iter()
        .any(|v| *v != [245, 245, 245, 255]));
    let mut model = lib3mf::Model::new();
    let mut mesh = lib3mf::Mesh::new();
    mesh.vertices = vec![
        lib3mf::Vertex::new(0., 0., 0.),
        lib3mf::Vertex::new(1., 0., 0.),
        lib3mf::Vertex::new(0., 1., 0.),
    ];
    mesh.triangles.push(lib3mf::Triangle::new(0, 1, 2));
    let mut object = lib3mf::Object::new(1);
    object.mesh = Some(mesh);
    model.resources.objects.push(object);
    let mut group = lib3mf::Object::new(2);
    group.components.push(lib3mf::Component::with_transform(
        1,
        [1., 0., 0., 0., 1., 0., 0., 0., 1., 2., 0., 0.],
    ));
    model.resources.objects.push(group);
    model.build.items.push(lib3mf::BuildItem::new(1));
    model.build.items.push(lib3mf::BuildItem::new(2));
    let file = tempfile::NamedTempFile::new().unwrap();
    model.write_to_file(file.path()).unwrap();
    let result = decode::render(
        file.path(),
        Format::ThreeMf,
        &Request::Mesh {
            yaw: 0.,
            pitch: 0.,
            width: 128,
        },
    )
    .unwrap();
    assert!(matches!(result.content, Content::Mesh { triangles: 2, .. }));
    assert_ne!(result.bytes, first.bytes);
}
#[test]
fn malformed_archives_and_worker_outputs_fail_before_viewer_use() {
    let file = source(b"PK\x03\x04broken");
    assert!(decode::render(
        file.path(),
        Format::Spreadsheet,
        &Request::Table {
            sheet: 0,
            row: 0,
            column: 0
        }
    )
    .is_err());
    let mut archive = zip::ZipWriter::new(Cursor::new(Vec::new()));
    archive
        .start_file("../outside", zip::write::SimpleFileOptions::default())
        .unwrap();
    archive.write_all(b"x").unwrap();
    let file = source(&archive.finish().unwrap().into_inner());
    assert!(decode::render(
        file.path(),
        Format::ThreeMf,
        &Request::Mesh {
            yaw: 0.,
            pitch: 0.,
            width: 128
        }
    )
    .is_err());
    let bad = Preview {
        content: Content::Image {
            width: u32::MAX,
            height: 1,
            pages: 1,
            index: 0,
        },
        bytes: Vec::new(),
    };
    assert_eq!(bad.validate(), Err(Error::Limit));
    assert!(Request::Mesh {
        yaw: f32::NAN,
        pitch: 0.,
        width: 128
    }
    .validate()
    .is_err());
}

#[cfg(target_os = "linux")]
#[test]
fn sandbox_renders_without_host_files_and_cancellation_stops_work() {
    use sigil_media::sandbox::{Input, Runtime};
    use std::sync::atomic::AtomicBool;
    let runtime = Runtime {
        worker: env!("CARGO_BIN_EXE_sigil-preview").into(),
        pdfium: None,
        office_libraries: None,
        office_language_data: None,
    };
    let mut input = Input::new().unwrap();
    input.write_all(b"synthetic").unwrap();
    let result = runtime
        .render(
            &input,
            Format::Text,
            &Request::Text { offset: 0 },
            &AtomicBool::new(false),
        )
        .unwrap();
    assert!(matches!(result.content,Content::Text {text,..} if text=="synthetic"));
    assert_eq!(
        runtime.render(
            &input,
            Format::Text,
            &Request::Text { offset: 0 },
            &AtomicBool::new(true)
        ),
        Err(Error::Cancelled)
    );
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("secret");
    std::fs::write(&marker, b"synthetic").unwrap();
    let worker = dir.path().join("worker");
    std::fs::write(&worker,format!("#!/bin/sh\n[ ! -e '{}' ] || exit 9\n[ -z \"$SSH_AUTH_SOCK\" ] || exit 10\n[ ! -e /home ] || exit 11\nexec /usr/bin/cat /input\n",marker.display())).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&worker, std::fs::Permissions::from_mode(0o700)).unwrap();
    let runtime = Runtime { worker, ..runtime };
    let mut input = Input::new().unwrap();
    Preview {
        content: Content::Text {
            text: "ok".into(),
            next: None,
        },
        bytes: Vec::new(),
    }
    .write(&mut input)
    .unwrap();
    assert!(matches!(Input::new(), Err(Error::Limit)));
    let result = runtime.render(
        &input,
        Format::Text,
        &Request::Text { offset: 0 },
        &AtomicBool::new(false),
    );
    assert!(result.is_ok(), "{result:?}");
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::fs::write(&runtime.worker,format!("#!/bin/sh\n/usr/bin/curl --silent --max-time 1 http://127.0.0.1:{port}/ >/dev/null 2>&1\nexec /usr/bin/cat /input\n")).unwrap();
    runtime
        .render(
            &input,
            Format::Text,
            &Request::Text { offset: 0 },
            &AtomicBool::new(false),
        )
        .unwrap();
    listener.set_nonblocking(true).unwrap();
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
    std::fs::write(&runtime.worker, "#!/bin/sh\n/usr/bin/sleep 60 &\nwait\n").unwrap();
    let cancel = std::sync::Arc::new(AtomicBool::new(false));
    let signal = cancel.clone();
    let setter = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(100));
        signal.store(true, std::sync::atomic::Ordering::Release);
    });
    let start = std::time::Instant::now();
    assert_eq!(
        runtime.render(&input, Format::Text, &Request::Text { offset: 0 }, &cancel),
        Err(Error::Cancelled)
    );
    setter.join().unwrap();
    assert!(start.elapsed() < std::time::Duration::from_secs(3));
}
