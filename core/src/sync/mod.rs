//! SyncService lifecycle → status.sync pushes.
pub mod members;
pub mod rooms;
pub mod settings;

use std::sync::Arc;

use matrix_sdk::Client;
use matrix_sdk_ui::sync_service::{State as SyncState, SyncService};
use tracing::info;

use crate::engine::SharedEngine;

pub async fn start(engine: SharedEngine, client: Client) {
    let sync = match SyncService::builder(client.clone()).with_offline_mode().build().await {
        Ok(s) => Arc::new(s),
        Err(e) => {
            engine.state.lock().sync_state = "error".into();
            engine.set_error(format!("sync service failed to build: {e}"));
            return;
        }
    };
    engine.state.lock().sync = Some(sync.clone());
    let mut states = sync.state();
    let engine2 = engine.clone();
    tokio::spawn(async move {
        while let Some(st) = states.next().await {
            let (name, err) = match &st {
                SyncState::Idle => ("idle", String::new()),
                SyncState::Running => ("running", String::new()),
                SyncState::Offline => ("offline", String::new()),
                SyncState::Terminated => ("terminated", String::new()),
                SyncState::Error(e) => ("error", e.to_string()),
            };
            info!("sync state: {name} {err}");
            {
                let mut s = engine2.state.lock();
                s.sync_state = name.into();
                s.sync_error = err.clone();
                if err.contains("M_UNRECOGNIZED") || err.contains("404") {
                    s.last_error = "the homeserver does not support simplified sliding sync (MSC4186)".into();
                }
            }
            engine2.broadcast_status();
        }
    });
    sync.start().await;
    rooms::start(engine.clone(), client.clone(), sync.clone());
    tokio::spawn(rooms::run_daily_refresh(engine.clone()));
    crate::presence::start(engine.clone(), client.clone());
    crate::geo::start(engine.clone());
    {
        let engine = engine.clone();
        tokio::spawn(async move { crate::timeline::beacon::reap(&engine).await });
    }
    {
        let engine = engine.clone();
        tokio::spawn(async move { crate::maps::refresh(&engine).await });
    }
}

pub async fn stop(engine: &SharedEngine) -> bool {
    let sync = engine.state.lock().sync.take();
    crate::timeline::close_all(engine).await;
    if let Some(s) = sync {
        s.stop().await;
        engine.state.lock().sync_state = "offline".into();
        true
    } else {
        false
    }
}
