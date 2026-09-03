//! Account setup: register the name, draw a credential and tokens, publish
//! key packages, and subscribe to the requests slot.

use crate::provider::SigilProvider;
use crate::{Link, State};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use openmls::prelude::*;
use openmls_basic_credential::SignatureKeyPair;
use sigil_protocol::encoding::Writer;
use sigil_protocol::identity::ContactCard;
use sigil_protocol::wire::{Request, Response};
use sigil_protocol::{names, token};
use tls_codec::{Deserialize as _, Serialize as _};

pub const CIPHERSUITE: Ciphersuite =
    Ciphersuite::MLS_128_DHKEMX25519_CHACHA20POLY1305_SHA256_Ed25519;

/// The OS RNG behind the trait generation blind RSA wants.
pub struct Rng;
impl rand_core10::TryRng for Rng {
    type Error = core::convert::Infallible;
    fn try_next_u32(&mut self) -> Result<u32, Self::Error> {
        Ok(rand::random())
    }
    fn try_next_u64(&mut self) -> Result<u64, Self::Error> {
        Ok(rand::random())
    }
    fn try_fill_bytes(&mut self, dst: &mut [u8]) -> Result<(), Self::Error> {
        rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, dst);
        Ok(())
    }
}
impl rand_core10::TryCryptoRng for Rng {}

pub fn contact_card(st: &State) -> Vec<u8> {
    let id = st.identity();
    ContactCard {
        username: st.username.clone(),
        identity_pub: id.public(),
        kem_pub: id.kem.public().to_vec(),
        slot_server: st.server(),
        flags: 0,
    }
    .sign(&id)
}

pub async fn register(link: &Link, st: &mut State, invite: &str) -> anyhow::Result<()> {
    let server = st.server();
    link.call(
        &server,
        &Request::NameRegister {
            card: contact_card(st),
            gate: invite.as_bytes().to_vec(),
            token: vec![],
        },
        None,
    )
    .await?;
    credential(link, st).await?;
    draw_tokens(link, st, 20).await
}

pub async fn credential(link: &Link, st: &mut State) -> anyhow::Result<()> {
    let server = st.server();
    let id = st.identity();
    let cv = token::Verifier::from_spki(&link.credential_key(&server).await?)
        .map_err(|e| anyhow::anyhow!("{e:?}"))?;
    let pending = cv
        .blind(&mut Rng, rand::random())
        .map_err(|e| anyhow::anyhow!("{e:?}"))?;
    let mut msg = b"sigil v1 credential".to_vec();
    msg.extend_from_slice(&pending.blinded);
    let sig = ed25519_dalek::Signer::sign(&id.signing, &msg).to_bytes();
    let resp = link
        .call(
            &server,
            &Request::TokenCredential {
                identity_pub: id.public(),
                sig,
                gate: vec![],
                blinded: pending.blinded.clone(),
            },
            None,
        )
        .await?;
    let Response::TokenCredential { blind_sig } = resp else {
        anyhow::bail!("unexpected")
    };
    let cred = cv
        .finalize(&pending, &blind_sig)
        .map_err(|e| anyhow::anyhow!("{e:?}"))?;
    st.credential = Some(hex::encode(cred.encode()));
    st.save()
}

pub async fn draw_tokens(link: &Link, st: &mut State, n: u16) -> anyhow::Result<()> {
    let server = st.server();
    let card = link.server_card(&server).await?;
    let verifier =
        token::Verifier::from_spki(&card.token_key).map_err(|e| anyhow::anyhow!("{e:?}"))?;
    let cred = hex::decode(
        st.credential
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("no credential"))?,
    )?;
    let pend: Vec<token::Pending> = (0..n)
        .map(|_| verifier.blind(&mut Rng, rand::random()).unwrap())
        .collect();
    let resp = link
        .call(
            &server,
            &Request::TokenIssue {
                credential: cred,
                blinded: pend.iter().map(|p| p.blinded.clone()).collect(),
            },
            None,
        )
        .await?;
    let Response::TokenIssue { blind_sigs } = resp else {
        anyhow::bail!("unexpected")
    };
    for (p, bs) in pend.iter().zip(blind_sigs) {
        st.tokens.push(hex::encode(
            verifier
                .finalize(p, &bs)
                .map_err(|e| anyhow::anyhow!("{e:?}"))?
                .encode(),
        ));
    }
    st.save()
}

/// Keep the wallet topped up: below `min`, draw `batch` more.
pub async fn ensure_tokens(
    link: &Link,
    st: &mut State,
    min: usize,
    batch: u16,
) -> anyhow::Result<()> {
    if st.tokens.len() >= min || st.credential.is_none() {
        return Ok(());
    }
    draw_tokens(link, st, batch).await
}

/// The MLS credential for this device: SPE(identity_pub, device_pub, sig).
pub fn mls_credential(st: &State) -> (CredentialWithKey, SignatureKeyPair) {
    let id = st.identity();
    let dev = st.device_key();
    let device_pub = dev.verifying_key().to_bytes();
    let mut msg = b"sigil v1 device".to_vec();
    msg.extend_from_slice(&device_pub);
    let sig = ed25519_dalek::Signer::sign(&id.signing, &msg).to_bytes();
    let identity = Writer::new()
        .fixed(&id.public())
        .fixed(&device_pub)
        .fixed(&sig)
        .finish();
    let signer = SignatureKeyPair::from_raw(
        SignatureScheme::ED25519,
        dev.to_bytes().to_vec(),
        device_pub.to_vec(),
    );
    let credential = BasicCredential::new(identity);
    (
        CredentialWithKey {
            credential: credential.into(),
            signature_key: signer.to_public_vec().into(),
        },
        signer,
    )
}

/// Parse and check a peer's MLS credential against the card we hold for them.
pub fn check_credential(
    cred: &Credential,
    expected_identity: &[u8; 32],
) -> anyhow::Result<[u8; 32]> {
    let basic = BasicCredential::try_from(cred.clone())
        .map_err(|_| anyhow::anyhow!("not a basic credential"))?;
    let b = basic.identity();
    if b.len() != 128 {
        anyhow::bail!("bad credential length");
    }
    let identity_pub: [u8; 32] = b[..32].try_into().unwrap();
    let device_pub: [u8; 32] = b[32..64].try_into().unwrap();
    let sig: [u8; 64] = b[64..].try_into().unwrap();
    if &identity_pub != expected_identity {
        anyhow::bail!("credential identity does not match the contact card");
    }
    let mut msg = b"sigil v1 device".to_vec();
    msg.extend_from_slice(&device_pub);
    let vk = ed25519_dalek::VerifyingKey::from_bytes(&identity_pub)?;
    ed25519_dalek::Verifier::verify(&vk, &msg, &ed25519_dalek::Signature::from_bytes(&sig))?;
    Ok(device_pub)
}

/// Build `n` key packages, seal each under the shelf key, and publish.
pub async fn publish_key_packages(
    link: &Link,
    st: &mut State,
    provider: &SigilProvider,
    n: u16,
) -> anyhow::Result<()> {
    let id = st.identity();
    let (cred, signer) = mls_credential(st);
    let shelf = names::shelf_address(&id.public());
    let key = names::shelf_key(&id.public());
    let mut ad = b"sigil v1 shelf".to_vec();
    ad.extend_from_slice(&shelf);
    let mut w = Writer::new().u16(n);
    for _ in 0..n {
        let kp = KeyPackage::builder()
            .build(CIPHERSUITE, provider, &signer, cred.clone())
            .map_err(|e| anyhow::anyhow!("{e:?}"))?;
        let bytes = kp.key_package().tls_serialize_detached()?;
        let nonce: [u8; 24] = rand::random();
        let ct = XChaCha20Poly1305::new((&key).into())
            .encrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: &bytes,
                    aad: &ad,
                },
            )
            .map_err(|_| anyhow::anyhow!("seal"))?;
        let mut sealed = nonce.to_vec();
        sealed.extend_from_slice(&ct);
        w = w.bytes(&sealed);
    }
    provider.save()?;
    let sealed_list = w.finish();
    let mut msg = b"sigil v1 shelf put".to_vec();
    msg.extend_from_slice(&shelf);
    msg.extend_from_slice(&sealed_list);
    let sig = ed25519_dalek::Signer::sign(&id.signing, &msg).to_bytes();
    let token = st.take_token()?;
    link.call(
        &st.server(),
        &Request::ShelfPut {
            shelf,
            sealed: sealed_list,
            identity_pub: id.public(),
            sig,
            token,
        },
        None,
    )
    .await?;
    Ok(())
}

/// Fetch one of `card`'s key packages and open it.
pub async fn take_key_package(
    link: &Link,
    provider: &SigilProvider,
    card: &ContactCard,
) -> anyhow::Result<KeyPackage> {
    let shelf = names::shelf_address(&card.identity_pub);
    let key = names::shelf_key(&card.identity_pub);
    let resp = link
        .call(&card.slot_server, &Request::ShelfTake { shelf }, None)
        .await?;
    let Response::ShelfTake { sealed } = resp else {
        anyhow::bail!("unexpected")
    };
    if sealed.len() < 40 {
        anyhow::bail!("short package");
    }
    let mut ad = b"sigil v1 shelf".to_vec();
    ad.extend_from_slice(&shelf);
    let plain = XChaCha20Poly1305::new((&key).into())
        .decrypt(
            XNonce::from_slice(&sealed[..24]),
            Payload {
                msg: &sealed[24..],
                aad: &ad,
            },
        )
        .map_err(|_| anyhow::anyhow!("cannot open key package"))?;
    let kp_in = KeyPackageIn::tls_deserialize_exact(&plain)?;
    let kp = kp_in
        .validate(provider.crypto(), ProtocolVersion::Mls10)
        .map_err(|e| anyhow::anyhow!("{e:?}"))?;
    check_credential(kp.leaf_node().credential(), &card.identity_pub)?;
    Ok(kp)
}

/// Subscribe to this account's requests slot for the current and next period.
pub async fn subscribe_requests(link: &Link, st: &mut State) -> anyhow::Result<Vec<[u8; 32]>> {
    let id = st.identity();
    let server = st.server();
    let period = (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs()
        / 2_592_000) as u32;
    let mut handles = Vec::new();
    for p in [period, period + 1] {
        let address = names::requests_address(&id.public(), p);
        let nonce = link.nonce(&server).await?;
        let proof = names::requests_read_proof(&id.signing, &address, &nonce);
        let handle: [u8; 32] = rand::random();
        let token = st.take_token()?;
        link.call(
            &server,
            &Request::SlotSubscribe {
                address,
                wake_handle: handle,
                proof: proof.to_vec(),
                token,
            },
            Some(handle),
        )
        .await?;
        handles.push(handle);
    }
    Ok(handles)
}

pub async fn lookup(link: &Link, username: &str) -> anyhow::Result<ContactCard> {
    let (local, server) =
        names::parse_username(username).map_err(|_| anyhow::anyhow!("bad username"))?;
    let resp = link
        .call(
            server,
            &Request::NameLookup {
                localpart: local.to_string(),
            },
            None,
        )
        .await?;
    let Response::Bytes(b) = resp else {
        anyhow::bail!("unexpected")
    };
    let card = ContactCard::verify(&b).map_err(|e| anyhow::anyhow!("{e:?}"))?;
    if card.username != username {
        anyhow::bail!("card is for {} not {username}", card.username);
    }
    Ok(card)
}

fn http_client(proxy: Option<&str>, secs: u64) -> anyhow::Result<reqwest::Client> {
    let mut http = reqwest::Client::builder().timeout(std::time::Duration::from_secs(secs));
    if let Some(px) = proxy.filter(|p| !p.is_empty()) {
        http = http.proxy(reqwest::Proxy::all(format!("socks5h://{px}"))?);
    }
    Ok(http.build()?)
}

/// `https://` everywhere but loopback, which is plain HTTP (tests).
pub fn scheme_for(host: &str) -> &'static str {
    let h = host.split(':').next().unwrap_or(host);
    if h == "localhost" || h.starts_with("127.") || h == "[::1]" {
        "http"
    } else {
        "https"
    }
}

/// The name people use (`sigil.example`) does not have to be where the
/// server answers: `https://<name>/.well-known/sigil` may say
/// `{"server": "host[:port]"}` and the base URL moves there. Without a
/// pointer the name is the address. A name with a scheme is taken as is
/// (a test server). The lookup goes through the proxy when one is set.
///
/// `SIGIL_TEST_HOSTS` (`name=http://127.0.0.1:port,…`) tells the lookup
/// where a test name's pointer lives, since tests have no DNS.
pub async fn resolve(server: &str, proxy: Option<&str>) -> anyhow::Result<String> {
    let server = server.trim().trim_end_matches('/');
    if server.contains("://") {
        return Ok(server.to_string());
    }
    let plain = format!("{}://{server}", scheme_for(server));
    let lookup = std::env::var("SIGIL_TEST_HOSTS")
        .ok()
        .and_then(|v| {
            v.split(',')
                .filter_map(|e| e.split_once('='))
                .find(|(n, _)| n.trim() == server)
                .map(|(_, u)| u.trim().to_string())
        })
        .unwrap_or_else(|| plain.clone());
    let pointer = async {
        let r = http_client(proxy, 5)?
            .get(format!("{lookup}/.well-known/sigil"))
            .send()
            .await?;
        if !r.status().is_success() {
            return Ok::<Option<String>, anyhow::Error>(None);
        }
        let v: serde_json::Value = serde_json::from_slice(&r.bytes().await?)?;
        Ok(v.get("server")
            .and_then(|s| s.as_str())
            .map(|s| s.trim().trim_end_matches('/').to_string())
            .filter(|s| !s.is_empty() && !s.contains('/')))
    };
    match pointer.await {
        Ok(Some(at)) => Ok(format!("{}://{at}", scheme_for(&at))),
        _ => Ok(plain),
    }
}

/// Ask a server what it offers before there is an account or a link: its
/// signed card, fetched straight from `<base>/info` (through the SOCKS
/// proxy when one is set), where the base comes from [`resolve`]. This is
/// the one request that goes to the server directly rather than through
/// an Envoy, and it carries nothing about who is asking. Returns the card
/// and the base URL the name resolved to.
pub async fn probe(
    server: &str,
    proxy: Option<&str>,
) -> anyhow::Result<(sigil_protocol::wire::ServerCard, String)> {
    let server = server.trim().trim_end_matches('/');
    let base = resolve(server, proxy).await?;
    let bytes = http_client(proxy, 15)?
        .get(format!("{base}/info"))
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;
    if bytes.len() < 64 {
        anyhow::bail!("{server} did not answer with a server card");
    }
    let body = &bytes[..bytes.len() - 64];
    let card =
        sigil_protocol::wire::ServerCard::decode(body).map_err(|e| anyhow::anyhow!("{e:?}"))?;
    let mut msg = b"sigil v1 server card".to_vec();
    msg.extend_from_slice(body);
    let vk = ed25519_dalek::VerifyingKey::from_bytes(&card.signing_pub)?;
    ed25519_dalek::Verifier::verify(
        &vk,
        &msg,
        &ed25519_dalek::Signature::from_slice(&bytes[bytes.len() - 64..])?,
    )?;
    Ok((card, base))
}

/// The Envoy address for a server name: its base, as a WebSocket, `/envoy`.
pub async fn envoy_for(server: &str, proxy: Option<&str>) -> anyhow::Result<String> {
    let base = resolve(server, proxy).await?;
    Ok(base.replacen("http", "ws", 1) + "/envoy")
}
