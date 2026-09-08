use sigil_server::{
    auth::AdminToken,
    store::{lock_directory, Store},
};
use std::{
    env,
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    path::PathBuf,
    time::Duration,
};

#[tokio::main]
async fn main() {
    if let Err(message) = run().await {
        eprintln!("{message}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), &'static str> {
    let mut args = env::args().skip(1);
    let command = args.next().unwrap_or_else(|| "serve".into());
    if command == "healthcheck" {
        if args.next().is_some() {
            return Err("unexpected arguments");
        }
        let mut address = listen_address()?;
        if address.ip().is_unspecified() {
            address.set_ip(if address.is_ipv4() {
                std::net::Ipv4Addr::LOCALHOST.into()
            } else {
                std::net::Ipv6Addr::LOCALHOST.into()
            });
        }
        let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(2))
            .map_err(|_| "health check connection failed")?;
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .map_err(|_| "health check setup failed")?;
        stream
            .set_write_timeout(Some(Duration::from_secs(2)))
            .map_err(|_| "health check setup failed")?;
        stream
            .write_all(b"GET /healthz HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .map_err(|_| "health check write failed")?;
        let mut status = [0u8; 12];
        stream
            .read_exact(&mut status)
            .map_err(|_| "health check read failed")?;
        return if &status == b"HTTP/1.1 200" {
            Ok(())
        } else {
            Err("health check failed")
        };
    }
    if command == "--help" {
        println!("sigil-server [serve|backup DESTINATION|restore SOURCE|rotate-admin-token|reset-admin-login]\nSet SIGIL_DATA_DIR to a private storage directory.\nSIGIL_LISTEN defaults to 127.0.0.1:8080. Non-loopback HTTP requires a trusted TLS reverse proxy.\nBackup, restore, and token rotation require the server to be stopped.");
        return Ok(());
    }
    if ![
        "serve",
        "backup",
        "restore",
        "rotate-admin-token",
        "reset-admin-login",
    ]
    .contains(&command.as_str())
    {
        return Err("unknown command; use --help");
    }
    let operand = if command == "backup" || command == "restore" {
        Some(PathBuf::from(args.next().ok_or("missing backup path")?))
    } else {
        None
    };
    if args.next().is_some() {
        return Err("unexpected arguments");
    }
    let directory = PathBuf::from(
        env::var_os("SIGIL_DATA_DIR").ok_or("set SIGIL_DATA_DIR outside the repository")?,
    );
    let _lock = lock_directory(&directory).map_err(|_| {
        "cannot lock private data directory; check permissions and stop other instances"
    })?;
    let database = directory.join("sigil.db");
    let token_path = directory.join("admin.token");
    if command == "restore" {
        if database.exists() || token_path.exists() {
            return Err("restore requires a new empty data directory");
        }
        Store::restore(operand.as_deref().ok_or("missing restore source")?, &database).map_err(|_| "restore failed; source must be a valid compatible backup and destination must not exist")?;
        AdminToken::load_or_create(&token_path)
            .map_err(|_| "could not create new admin credential")?;
        println!("Server metadata restored; device credentials invalidated and a new Admin credential created.");
        return Ok(());
    }
    if command == "rotate-admin-token" {
        if !database.exists() {
            return Err("server storage has not been initialized");
        }
        AdminToken::rotate(&token_path).map_err(|_| "admin token rotation failed")?;
        println!("Admin credential rotated.");
        return Ok(());
    }
    if command == "serve" {
        sigil_server::operations::activate_restore(&directory)
            .map_err(|_| "staged restore activation failed; original data is preserved")?;
    }
    let mut store = Store::open(&database).map_err(|_| "cannot open compatible server storage")?;
    if command == "reset-admin-login" {
        store
            .web_reset_login()
            .map_err(|_| "administrator login reset failed")?;
        println!(
            "Browser sessions revoked. Restart the server and use its new one-time setup code."
        );
        return Ok(());
    }
    if command == "backup" {
        store
            .backup(operand.as_deref().ok_or("missing backup destination")?)
            .map_err(|_| "backup failed; destination must not exist")?;
        println!("Server metadata backup complete; installation Admin credential is excluded.");
        return Ok(());
    }
    let address = listen_address()?;
    let token = AdminToken::load_or_create(&token_path)
        .map_err(|_| "cannot load private admin credential")?;
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .map_err(|_| "cannot bind listener")?;
    println!("Sigil started. Bootstrap Admin credential is in the private data directory.");
    if let Some(code) = store
        .web_setup_code()
        .map_err(|_| "cannot initialize browser setup")?
    {
        println!("One-time browser setup code: {code}");
    }
    let (router, maintenance) = sigil_server::router_with_maintenance(store, token);
    tokio::select! {
        result = async { axum::serve(listener, router).with_graceful_shutdown(shutdown()).await } => result.map_err(|_| "server failed"),
        _ = maintenance => Err("storage maintenance stopped unexpectedly"),
    }
}

fn listen_address() -> Result<SocketAddr, &'static str> {
    env::var("SIGIL_LISTEN")
        .unwrap_or_else(|_| "127.0.0.1:8080".into())
        .parse()
        .map_err(|_| "invalid SIGIL_LISTEN address")
}

async fn shutdown() {
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .expect("install termination handler");
    tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = terminate.recv() => {} }
}
