//! Startpunkt des Dienstes.

use std::sync::Arc;

use alkis_proxy::api::{self, metrics::Metrics, AppState};
use alkis_proxy::cache::{RedisStore, TileCache};
use alkis_proxy::config::{states, Settings};
use alkis_proxy::upstream::{CircuitBreaker, UpstreamClient};
use tower_http::compression::CompressionLayer;
use tower_http::trace::TraceLayer;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "alkis_proxy=info,tower_http=warn".into()),
        )
        .init();

    let settings = Settings::from_env();

    // Der Cache ist optional: Ohne Redis läuft der Dienst langsamer, aber
    // korrekt weiter. Ein Cache-Ausfall darf kein Dienstausfall sein.
    let store = match &settings.redis_url {
        Some(url) => match RedisStore::connect(url).await {
            Ok(s) => {
                tracing::info!("Cache angebunden");
                Some(s)
            }
            Err(e) => {
                tracing::warn!(error = %e, "Redis nicht erreichbar — Dienst läuft ohne Cache");
                None
            }
        },
        None => {
            tracing::warn!("ALKIS_REDIS_URL nicht gesetzt — Dienst läuft ohne Cache");
            None
        }
    };

    let configured = states::all().iter().filter(|s| s.endpoint.is_some()).count();
    let state = Arc::new(AppState {
        client: UpstreamClient::new(settings.upstream_timeout)?,
        cache: TileCache::new(store),
        breaker: CircuitBreaker::new(),
        metrics: Metrics::default(),
        settings: settings.clone(),
    });

    // Kompression spart bei großen FeatureCollections viel Bandbreite, kann
    // aber bei manchen Clients Probleme machen — deshalb abschaltbar.
    let mut app = api::router(state).layer(TraceLayer::new_for_http());
    if std::env::var("ALKIS_DISABLE_COMPRESSION").is_err() {
        app = app.layer(CompressionLayer::new());
    } else {
        tracing::info!("Antwortkompression ist abgeschaltet");
    }

    let listener = tokio::net::TcpListener::bind(&settings.bind).await?;
    tracing::info!(
        adresse = %settings.bind,
        bundeslaender = configured,
        "alkis-proxy {} bereit",
        env!("CARGO_PKG_VERSION")
    );

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    tracing::info!("beendet");
    Ok(())
}

/// Wartet auf Ctrl-C oder SIGTERM, damit der Container sauber herunterfährt.
async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c().await.expect("Ctrl-C-Handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("SIGTERM-Handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
    tracing::info!("Signal empfangen, fahre herunter");
}
