//! Groups: creation with several members, invites, renames, leaving, and
//! the policy snapshot every member keeps in step. A direct message is a
//! group of two with no name; everything here works for it too.

use crate::account::{lookup, mls_credential, take_key_package};
use crate::conversation::{
    self, create_config, epoch_material, load_group, record_epoch, send_event,
};
use crate::provider::SigilProvider;
use crate::state::{Conversation, Member};
use crate::{Link, State};
use openmls::prelude::*;
use serde::{Deserialize, Serialize};
use sigil_protocol::encoding::Writer;
use sigil_protocol::identity::ContactCard;
use sigil_protocol::wire::Request;
use sigil_protocol::{envelope, epoch, names, requests};

/// The policy snapshot, sent as an event of kind 8 whenever it changes and
/// carried in every Welcome.
#[derive(Serialize, Deserialize, Clone, Default)]
pub struct Policy {
    pub name: String,
    pub members: Vec<Member>,
    pub admins: Vec<String>,
}

impl Policy {
    pub fn from_conv(c: &Conversation) -> Policy {
        Policy {
            name: c.name.clone(),
            members: c.members.clone(),
            admins: c.admins.clone(),
        }
    }
    pub fn apply(&self, c: &mut Conversation, me: &str) {
        c.name = self.name.clone();
        c.members = self.members.clone();
        c.admins = self.admins.clone();
        c.peers = self
            .members
            .iter()
            .filter(|m| m.username != me)
            .map(|m| m.username.clone())
            .collect();
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

/// Write a Welcome plus the policy into `card`'s requests slot.
async fn send_welcome(
    link: &Link,
    st: &mut State,
    card: &ContactCard,
    welcome: &MlsMessageOut,
    policy: &Policy,
    first_text: &str,
) -> anyhow::Result<()> {
    let body = Writer::new()
        .bytes(&welcome.to_bytes()?)
        .bytes(&crate::account::contact_card(st))
        .str(first_text)
        .bytes(&serde_json::to_vec(policy)?)
        .finish();
    let ev = envelope::Event {
        kind: envelope::Kind::Welcome as u16,
        ts_ms: now_ms(),
        reference: vec![],
        body,
    };
    let period = (now_ms() / 1000 / 2_592_000) as u32;
    let address = names::requests_address(&card.identity_pub, period);
    let sealed = requests::seal(
        &card.kem_pub,
        &address,
        &rand::random(),
        &rand::random(),
        &ev.encode(),
    )
    .map_err(|e| anyhow::anyhow!("{e:?}"))?;
    let token = st.take_token()?;
    link.call(
        &card.slot_server,
        &Request::RequestsPut {
            address,
            envelope: sealed,
            token,
        },
        None,
    )
    .await?;
    Ok(())
}

/// Write a commit to the conversation's current slot and merge it.
async fn commit(
    link: &Link,
    st: &mut State,
    provider: &SigilProvider,
    conv: &Conversation,
    group: &mut MlsGroup,
    commit: &MlsMessageOut,
) -> anyhow::Result<()> {
    let ep = epoch_material(group, provider)?;
    record_epoch(st, conv, &ep);
    let sealed = envelope::seal(
        &ep.envelope_key,
        &ep.address,
        &rand::random(),
        &commit.to_bytes()?,
    )
    .map_err(|e| anyhow::anyhow!("{e:?}"))?;
    let sig = epoch::sign_put(&ep.write_key, &ep.address, &sealed);
    let token = st.take_token()?;
    let resp = link
        .call(
            &conv.slot_server,
            &Request::SlotPut {
                address: ep.address,
                write_pub: ep.write_pub,
                sig,
                envelope: sealed,
                token,
            },
            None,
        )
        .await?;
    if let sigil_protocol::wire::Response::SlotPut { seq } = resp {
        if let Some(c) = st
            .conversations
            .iter_mut()
            .find(|c| c.group_id == conv.group_id)
        {
            c.sent.insert(
                format!("{}:{seq}", hex::encode(ep.address)),
                format!("{}\u{1f}", now_ms()),
            );
        }
        conversation::set_cursor(st, conv, &ep.address, seq);
    }
    group
        .merge_pending_commit(provider)
        .map_err(|e| anyhow::anyhow!("{e:?}"))?;
    provider.save()?;
    st.save()?;
    Ok(())
}

/// Create a group with `name` and invite `usernames`. Each invitee gets a
/// Welcome in their requests slot; the policy is the first message.
pub async fn create(
    link: &Link,
    st: &mut State,
    provider: &SigilProvider,
    name: &str,
    usernames: &[String],
    first_text: &str,
) -> anyhow::Result<Conversation> {
    let me = st.username.clone();
    let my_identity = hex::encode(st.identity().public());
    let mut cards = Vec::new();
    for u in usernames {
        cards.push(lookup(link, u).await?);
    }
    let mut kps = Vec::new();
    for c in &cards {
        kps.push(take_key_package(link, provider, c).await?);
    }
    let (cred, signer) = mls_credential(st);
    let group_id = GroupId::from_slice(&rand::random::<[u8; 32]>());
    let mut group =
        MlsGroup::new_with_group_id(provider, &signer, &create_config(), group_id.clone(), cred)
            .map_err(|e| anyhow::anyhow!("{e:?}"))?;
    let mut members = vec![Member {
        username: me.clone(),
        identity: my_identity.clone(),
    }];
    members.extend(cards.iter().map(|c| Member {
        username: c.username.clone(),
        identity: hex::encode(c.identity_pub),
    }));
    let policy = Policy {
        name: name.to_string(),
        members,
        admins: vec![my_identity],
    };
    let welcome = if kps.is_empty() {
        None
    } else {
        let (_commit, welcome, _) = group
            .add_members(provider, &signer, &kps)
            .map_err(|e| anyhow::anyhow!("{e:?}"))?;
        group
            .merge_pending_commit(provider)
            .map_err(|e| anyhow::anyhow!("{e:?}"))?;
        Some(welcome)
    };
    provider.save()?;
    let mut conv = Conversation {
        group_id: hex::encode(group_id.as_slice()),
        slot_server: st.server(),
        ..Default::default()
    };
    policy.apply(&mut conv, &me);
    st.conversations.push(conv.clone());
    st.save()?;
    if let Some(w) = &welcome {
        for c in &cards {
            send_welcome(link, st, c, w, &policy, first_text).await?;
        }
    }
    // A group announces its policy as the first message; a direct message
    // needs no announcement, the Welcome already carries it.
    if !(name.is_empty() && usernames.len() == 1) {
        send_event(
            link,
            st,
            provider,
            &conv,
            envelope::Kind::Policy,
            &[],
            &serde_json::to_vec(&policy)?,
        )
        .await?;
    }
    Ok(conv)
}

/// Add `username` to an existing conversation.
pub async fn invite(
    link: &Link,
    st: &mut State,
    provider: &SigilProvider,
    conv: &Conversation,
    username: &str,
) -> anyhow::Result<()> {
    if conv.members.iter().any(|m| m.username == username) {
        anyhow::bail!("{username} is already in the conversation");
    }
    let card = lookup(link, username).await?;
    let kp = take_key_package(link, provider, &card).await?;
    // make sure we are on the latest epoch first
    let _ = conversation::catch_up(link, st, provider, conv).await?;
    let mut group = load_group(provider, conv)?;
    let (_, signer) = mls_credential(st);
    let (c, welcome, _) = group
        .add_members(provider, &signer, std::slice::from_ref(&kp))
        .map_err(|e| anyhow::anyhow!("{e:?}"))?;
    commit(link, st, provider, conv, &mut group, &c).await?;
    let mut policy = Policy::from_conv(conv);
    policy.members.push(Member {
        username: username.to_string(),
        identity: hex::encode(card.identity_pub),
    });
    let me = st.username.clone();
    if let Some(cc) = st
        .conversations
        .iter_mut()
        .find(|c| c.group_id == conv.group_id)
    {
        policy.apply(cc, &me);
    }
    st.save()?;
    send_welcome(link, st, &card, &welcome, &policy, "").await?;
    let conv2 = st
        .conversations
        .iter()
        .find(|c| c.group_id == conv.group_id)
        .cloned()
        .unwrap();
    send_event(
        link,
        st,
        provider,
        &conv2,
        envelope::Kind::Policy,
        &[],
        &serde_json::to_vec(&policy)?,
    )
    .await?;
    Ok(())
}

/// Rename: a new policy snapshot.
pub async fn rename(
    link: &Link,
    st: &mut State,
    provider: &SigilProvider,
    conv: &Conversation,
    name: &str,
) -> anyhow::Result<()> {
    let mut policy = Policy::from_conv(conv);
    policy.name = name.to_string();
    let me = st.username.clone();
    if let Some(cc) = st
        .conversations
        .iter_mut()
        .find(|c| c.group_id == conv.group_id)
    {
        policy.apply(cc, &me);
    }
    st.save()?;
    let conv2 = st
        .conversations
        .iter()
        .find(|c| c.group_id == conv.group_id)
        .cloned()
        .unwrap();
    send_event(
        link,
        st,
        provider,
        &conv2,
        envelope::Kind::Policy,
        &[],
        &serde_json::to_vec(&policy)?,
    )
    .await?;
    Ok(())
}

/// Leave: tell the others, then forget. The lowest remaining identity
/// removes our leaves (`on_left`).
pub async fn leave(
    link: &Link,
    st: &mut State,
    provider: &SigilProvider,
    conv: &Conversation,
) -> anyhow::Result<()> {
    let body = serde_json::to_vec(
        &serde_json::json!({"action":"leave","username": st.username, "identity": hex::encode(st.identity().public())}),
    )?;
    let _ = send_event(
        link,
        st,
        provider,
        conv,
        envelope::Kind::Membership,
        &[],
        &body,
    )
    .await;
    st.conversations.retain(|c| c.group_id != conv.group_id);
    st.save()?;
    Ok(())
}

/// Someone said they left: drop them from the policy, and if we are the
/// lowest remaining identity, commit their removal so the epoch moves on
/// without them. Returns true if we committed.
pub async fn on_left(
    link: &Link,
    st: &mut State,
    provider: &SigilProvider,
    conv: &Conversation,
    identity_hex: &str,
) -> anyhow::Result<bool> {
    let me_hex = hex::encode(st.identity().public());
    let me = st.username.clone();
    let mut policy = Policy::from_conv(conv);
    policy.members.retain(|m| m.identity != identity_hex);
    policy.admins.retain(|a| a != identity_hex);
    if let Some(cc) = st
        .conversations
        .iter_mut()
        .find(|c| c.group_id == conv.group_id)
    {
        policy.apply(cc, &me);
    }
    st.save()?;
    let lowest = policy
        .members
        .iter()
        .map(|m| m.identity.clone())
        .min()
        .unwrap_or_default();
    if lowest != me_hex {
        return Ok(false);
    }
    let mut group = load_group(provider, conv)?;
    let identity: [u8; 32] = hex::decode(identity_hex)?
        .try_into()
        .map_err(|_| anyhow::anyhow!("bad identity"))?;
    let leaves: Vec<LeafNodeIndex> = group
        .members()
        .filter(|m| crate::account::check_credential(&m.credential, &identity).is_ok())
        .map(|m| m.index)
        .collect();
    if leaves.is_empty() {
        return Ok(false);
    }
    let (_, signer) = mls_credential(st);
    let (c, _, _) = group
        .remove_members(provider, &signer, &leaves)
        .map_err(|e| anyhow::anyhow!("{e:?}"))?;
    commit(link, st, provider, conv, &mut group, &c).await?;
    Ok(true)
}

/// What a control event changed.
pub enum Change {
    /// Name or members changed; the conversation is updated in `st`.
    Policy,
    /// `username` left; we may have committed their removal.
    Left {
        username: String,
        committed: bool,
    },
    None,
}

/// Apply a policy (kind 8) or membership (kind 7) event from a peer.
pub async fn apply_control(
    link: &Link,
    st: &mut State,
    provider: &SigilProvider,
    conv: &Conversation,
    kind: u16,
    from_identity: &[u8; 32],
    body: &str,
) -> anyhow::Result<Change> {
    let me = st.username.clone();
    if kind == envelope::Kind::Policy as u16 {
        let policy: Policy = serde_json::from_str(body)?;
        // only an admin, or the creator of a fresh conversation, may set policy
        let from = hex::encode(from_identity);
        if !conv.admins.is_empty() && !conv.admins.contains(&from) {
            anyhow::bail!("policy from a non-admin ignored");
        }
        if let Some(cc) = st
            .conversations
            .iter_mut()
            .find(|c| c.group_id == conv.group_id)
        {
            policy.apply(cc, &me);
        }
        st.save()?;
        return Ok(Change::Policy);
    }
    if kind == envelope::Kind::Membership as u16 {
        let v: serde_json::Value = serde_json::from_str(body)?;
        if v.get("action").and_then(|a| a.as_str()) == Some("leave") {
            let identity = v
                .get("identity")
                .and_then(|i| i.as_str())
                .unwrap_or("")
                .to_string();
            let username = v
                .get("username")
                .and_then(|u| u.as_str())
                .unwrap_or("")
                .to_string();
            if identity != hex::encode(from_identity) {
                anyhow::bail!("leave notice for someone else ignored");
            }
            let committed = on_left(link, st, provider, conv, &identity)
                .await
                .unwrap_or(false);
            return Ok(Change::Left {
                username,
                committed,
            });
        }
    }
    Ok(Change::None)
}
