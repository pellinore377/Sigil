//! `sigil-engine cli <req> k=v k:=json [--follow]`
use std::path::PathBuf;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

pub async fn run(path: PathBuf, req: String, params: Vec<String>, follow: bool) -> anyhow::Result<()> {
    let mut obj = serde_json::Map::new();
    obj.insert("req".into(), req.into());
    obj.insert("id".into(), 1.into());
    for p in params {
        if let Some((k, v)) = p.split_once(":=") {
            obj.insert(k.into(), serde_json::from_str(v)?);
        } else if let Some((k, v)) = p.split_once('=') {
            obj.insert(k.into(), v.into());
        } else {
            anyhow::bail!("bad param '{p}': expected key=value or key:=json");
        }
    }
    let stream = UnixStream::connect(&path).await?;
    let (rd, mut wr) = stream.into_split();
    wr.write_all(serde_json::Value::Object(obj).to_string().as_bytes()).await?;
    wr.write_all(b"\n").await?;
    let mut lines = BufReader::new(rd).lines();
    while let Some(line) = lines.next_line().await? {
        let v: serde_json::Value = serde_json::from_str(&line).unwrap_or(serde_json::Value::String(line.clone()));
        let is_reply = v.get("reply").is_some();
        if follow || is_reply {
            println!("{}", serde_json::to_string_pretty(&v)?);
        }
        if is_reply && !follow {
            break;
        }
    }
    Ok(())
}
