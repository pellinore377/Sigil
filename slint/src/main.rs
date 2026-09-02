//! Desktop dev runner. State is sandboxed away from the daily daemon's store:
//! two sync loops over one sqlite crypto store is how sessions get corrupted,
//! so this binary is always its own Matrix device. Set SIGIL_SLINT_SHARED=1
//! only if you know why that warning exists and disagree with it.

fn main() -> anyhow::Result<()> {
    if std::env::var_os("SIGIL_SLINT_SHARED").is_none() {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/".into());
        std::env::set_var("XDG_STATE_HOME", format!("{home}/.local/state/sigil-slint"));
        std::env::set_var("XDG_CACHE_HOME", format!("{home}/.cache/sigil-slint"));
    }
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    sigil_slint::run_app()
}
