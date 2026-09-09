use super::*;

fn local_urls(value: &mut Value) {
    match value {
        Value::String(text) if text.starts_with("/client/v0/maps/") => {
            *text = format!("https://sigil-map.invalid{text}")
        }
        Value::Array(values) => values.iter_mut().for_each(local_urls),
        Value::Object(values) => values.values_mut().for_each(local_urls),
        _ => (),
    }
}
impl ClientStore {
    pub fn mobile_map_resource(&self, path: &str) -> Result<Zeroizing<Vec<u8>>, Error> {
        if path.len() > 1024 || path.contains(['?', '#', '%', '\\', '\0']) {
            return Err(Error::InvalidEvent);
        }
        let path = path
            .strip_prefix("/client/v0/maps/")
            .ok_or(Error::InvalidEvent)?;
        let client = self.connected_client()?;
        let (bytes, content_type, encoding) = match path {
            "style.json" | "tiles.json" => {
                let mut value: Value = if path == "style.json" {
                    serde_json::from_slice(&client.map_style()?).map_err(|_| Error::InvalidEvent)?
                } else {
                    client.map_metadata()?
                };
                local_urls(&mut value);
                (
                    Zeroizing::new(serde_json::to_vec(&value).map_err(|_| Error::InvalidEvent)?),
                    "application/json".to_owned(),
                    None,
                )
            }
            _ if path.starts_with("assets/") => {
                let name = &path[7..];
                let kind = match name.rsplit('.').next() {
                    Some("json") => "application/json",
                    Some("pbf") => "application/x-protobuf",
                    Some("png") => "image/png",
                    _ => return Err(Error::InvalidEvent),
                };
                (client.map_asset(name)?, kind.into(), None)
            }
            _ if path.starts_with("tiles/") => {
                let v = path[6..].split('/').collect::<Vec<_>>();
                if v.len() != 3 {
                    return Err(Error::InvalidEvent);
                }
                let tile = client.map_tile(
                    v[0].parse().map_err(|_| Error::InvalidEvent)?,
                    v[1].parse().map_err(|_| Error::InvalidEvent)?,
                    v[2].parse().map_err(|_| Error::InvalidEvent)?,
                )?;
                let Some(tile) = tile else {
                    return Ok(Zeroizing::new(
                        b"204\napplication/octet-stream\n\n".to_vec(),
                    ));
                };
                (tile.bytes, tile.content_type, tile.encoding)
            }
            _ => return Err(Error::InvalidEvent),
        };
        let mut response = Zeroizing::new(
            format!("200\n{content_type}\n{}\n", encoding.unwrap_or_default()).into_bytes(),
        );
        response.extend_from_slice(&bytes);
        Ok(response)
    }
}
