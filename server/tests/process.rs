use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::Path,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

struct Server {
    child: Child,
    port: u16,
}
impl Server {
    fn start(directory: &Path) -> Self {
        let reservation = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = reservation.local_addr().unwrap().port();
        drop(reservation);
        let child = command(directory)
            .arg("serve")
            .env("SIGIL_LISTEN", format!("127.0.0.1:{port}"))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let server = Self { child, port };
        let deadline = Instant::now() + Duration::from_secs(5);
        while TcpStream::connect(("127.0.0.1", port)).is_err() {
            assert!(Instant::now() < deadline, "server did not listen");
            std::thread::sleep(Duration::from_millis(10));
        }
        server
    }
    fn request(&self, method: &str, path: &str, body: &str, token: Option<&str>) -> u16 {
        let mut connection = TcpStream::connect(("127.0.0.1", self.port)).unwrap();
        connection
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let authorization = token
            .map(|t| format!("Authorization: Bearer {t}\r\n"))
            .unwrap_or_default();
        write!(connection, "{method} {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{authorization}\r\n{body}", body.len()).unwrap();
        let mut response = String::new();
        connection.read_to_string(&mut response).unwrap();
        response.split_whitespace().nth(1).unwrap().parse().unwrap()
    }
    fn stop(mut self) {
        assert!(Command::new("kill")
            .args(["-TERM", &self.child.id().to_string()])
            .status()
            .unwrap()
            .success());
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert!(status.success());
                break;
            }
            assert!(Instant::now() < deadline, "server did not stop gracefully");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    fn crash(mut self) {
        self.child.kill().unwrap();
        assert!(!self.child.wait().unwrap().success());
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
fn command(directory: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_sigil-server"));
    command.env("SIGIL_DATA_DIR", directory);
    command
}

#[test]
fn actual_server_configuration_backup_restore_and_shutdown() {
    let temp = tempfile::tempdir().unwrap();
    let original = temp.path().join("original");
    let restored = temp.path().join("restored");
    let backup = temp.path().join("backup.db");
    let server = Server::start(&original);
    assert_eq!(server.request("GET", "/healthz", "", None), 200);
    assert_eq!(server.request("GET", "/readyz", "", None), 503);
    let token = std::fs::read_to_string(original.join("admin.token")).unwrap();
    assert_eq!(server.request("PUT", "/admin/v0/configuration", r#"{"expected_revision":0,"settings":{"server_name":"chat.example","default_quota_bytes":10737418240,"max_attachment_bytes":1073741824}}"#, Some(&token)), 200);
    assert!(!command(&original)
        .arg("backup")
        .arg(&backup)
        .output()
        .unwrap()
        .status
        .success());
    server.stop();
    assert!(command(&original)
        .arg("backup")
        .arg(&backup)
        .output()
        .unwrap()
        .status
        .success());
    assert!(command(&restored)
        .arg("restore")
        .arg(&backup)
        .output()
        .unwrap()
        .status
        .success());
    let server = Server::start(&restored);
    assert_eq!(server.request("GET", "/readyz", "", None), 200);
    assert_eq!(
        server.request("GET", "/admin/v0/configuration", "", Some(&token)),
        401
    );
    let new_token = std::fs::read_to_string(restored.join("admin.token")).unwrap();
    assert_eq!(
        server.request("GET", "/admin/v0/configuration", "", Some(&new_token)),
        200
    );
    server.stop();
}

#[test]
fn acknowledged_writes_survive_kill_restart_and_offline_restore() {
    use sha2::{Digest, Sha256};
    use sigil_protocol::{
        accounts::{Enrollment, InviteRequest},
        Configure,
    };
    use sigil_server::store::{lock_directory, Store};
    let directory = tempfile::tempdir().unwrap();
    let original = directory.path().join("original");
    let restored = directory.path().join("restored");
    let backup = directory.path().join("backup.db");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let lock = lock_directory(&original).unwrap();
    let mut store = Store::open(&original.join("sigil.db")).unwrap();
    store
        .configure(Configure {
            expected_revision: 0,
            settings: serde_json::from_str(r#"{"server_name":"chat.example"}"#).unwrap(),
        })
        .unwrap();
    let invitation = store
        .invite(
            InviteRequest {
                username: "synthetic".into(),
                expires_in_seconds: 60,
            },
            now,
        )
        .unwrap();
    let token = sigil_server::auth::random_secret().unwrap();
    let session = store
        .enroll(
            Enrollment {
                invitation: invitation.secret,
                device_credential: token.clone(),
                device_label: "Synthetic persistence test".into(),
            },
            now,
        )
        .unwrap();
    drop(store);
    drop(lock);
    let message = serde_json::json!({"recipient_device":session.device_id,"message_id":"12".repeat(32),"expires_at":now+3600,"payload":"ab".repeat(16)}).to_string();
    // Opaque synthetic bytes exercise server persistence, not client encryption.
    let object = [0u8; 36];
    let id: String = Sha256::digest(object)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let object_path = format!("/client/v0/recovery/objects/{id}");
    let head = serde_json::json!({"expected_generation":0,"expected_manifest":null,"manifest":id})
        .to_string();
    let server = Server::start(&original);
    let admin = std::fs::read_to_string(original.join("admin.token")).unwrap();
    assert_eq!(
        server.request("POST", "/client/v0/messages", &message, Some(&token)),
        202
    );
    assert_eq!(
        server.request(
            "PUT",
            &object_path,
            &serde_json::json!({"ciphertext":"00".repeat(36)}).to_string(),
            Some(&token)
        ),
        204
    );
    assert_eq!(
        server.request("PUT", "/client/v0/recovery/head", &head, Some(&token)),
        200
    );
    server.crash();
    let server = Server::start(&original);
    assert_eq!(
        server.request("GET", "/client/v0/session", "", Some(&token)),
        200
    );
    assert_eq!(
        server.request("POST", "/client/v0/messages", &message, Some(&token)),
        202
    );
    assert_eq!(
        server.request("PUT", "/client/v0/recovery/head", &head, Some(&token)),
        200
    );
    server.stop();
    let mut store = Store::open(&original.join("sigil.db")).unwrap();
    let mailbox = store.mailbox(&token, now).unwrap();
    assert_eq!(mailbox.len(), 1);
    assert_eq!(mailbox[0].payload, "ab".repeat(16));
    assert_eq!(store.recovery_head(&token, now).unwrap().generation, 1);
    drop(store);
    assert!(command(&original)
        .arg("backup")
        .arg(&backup)
        .output()
        .unwrap()
        .status
        .success());
    assert!(command(&restored)
        .arg("restore")
        .arg(&backup)
        .output()
        .unwrap()
        .status
        .success());
    let server = Server::start(&restored);
    assert_eq!(server.request("GET", "/readyz", "", None), 200);
    assert_eq!(
        server.request("GET", "/client/v0/session", "", Some(&token)),
        401
    );
    assert_eq!(
        server.request("GET", "/admin/v0/configuration", "", Some(&admin)),
        401
    );
    server.stop();
    let db = rusqlite::Connection::open(restored.join("sigil.db")).unwrap();
    assert_eq!(
        db.query_row(
            "SELECT data FROM recovery_objects WHERE id=?1",
            [&id],
            |r| r.get::<_, Vec<u8>>(0)
        )
        .unwrap(),
        object
    );
    assert_eq!(
        db.query_row("SELECT restored_checkpoint FROM recovery_heads", [], |r| {
            r.get::<_, i64>(0)
        })
        .unwrap(),
        1
    );
    assert_eq!(
        db.query_row("SELECT count(payload) FROM mailbox", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn running_server_expires_idle_payloads_in_background() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("idle");
    let lock = sigil_server::store::lock_directory(&data).unwrap();
    let store = sigil_server::store::Store::open(&data.join("sigil.db")).unwrap();
    let db = rusqlite::Connection::open(data.join("sigil.db")).unwrap();
    db.execute_batch("INSERT INTO accounts(id,username) VALUES('account','synthetic'); INSERT INTO devices(id,account_id,label,expires_at) VALUES('device','account','Synthetic',999999);
    WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<130)
    INSERT INTO mailbox(sender,message_id,recipient,payload,payload_hash,expires_at) SELECT 'device',printf('%064x',x),'device','abab',zeroblob(32),1 FROM n;").unwrap();
    drop(store);
    drop(lock);
    let server = Server::start(&data);
    let deadline = Instant::now() + Duration::from_secs(6);
    loop {
        let remaining: u32 = db
            .query_row(
                "SELECT count(*) FROM mailbox WHERE payload IS NOT NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        if remaining == 0 {
            break;
        }
        assert!(Instant::now() < deadline, "idle payloads were not expired");
        std::thread::sleep(Duration::from_millis(25));
    }
    assert_eq!(
        db.query_row("SELECT count(*) FROM mailbox", [], |r| r.get::<_, u32>(0))
            .unwrap(),
        130
    );
    server.stop();
}
