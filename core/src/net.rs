//! One place the engine builds HTTP clients, so every platform gets a
//! working certificate check. reqwest's rustls backend verifies through
//! the platform: fine on desktops, but on Android that means a Java
//! helper the app does not ship (it is a plain NativeActivity), and the
//! check fails before any request goes out. There the client carries
//! Mozilla's root store instead, the same roots the sigil-client crate
//! uses everywhere.

pub fn http_builder() -> reqwest::ClientBuilder {
    let b = reqwest::Client::builder();
    #[cfg(target_os = "android")]
    {
        let roots = rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let config = rustls::ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        b.use_preconfigured_tls(config)
    }
    #[cfg(not(target_os = "android"))]
    b
}
