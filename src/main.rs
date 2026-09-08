//! Startpunkt des Dienstes.

use std::sync::Arc;

use alkis_proxy::api::{self, metrics::Metrics, AppState};
use alkis_proxy::cache::{RedbStore, TileCache};
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

    // Der Cache ist optional: Ohne ihn läuft der Dienst langsamer, aber
    // korrekt weiter. Ein Cache-Ausfall darf kein Dienstausfall sein.
    let store = match &settings.cache_path {
        Some(path) => match RedbStore::open(path, settings.cache_max_bytes) {
            Ok(s) => {
                tracing::info!(pfad = %path.display(), "Cache angebunden");
                Some(s)
            }
            Err(e) => {
                tracing::warn!(error = %e, "Cache-Datei nicht nutzbar — Dienst läuft ohne Cache");
                None
            }
        },
        None => {
            tracing::warn!("ALKIS_CACHE_PATH nicht gesetzt — Dienst läuft ohne Cache");
            None
        }
    };

    let configured = states::all().iter().filter(|s| s.endpoint.is_some()).count();
    let state = Arc::new(AppState {
        client: UpstreamClient::new(settings.upstream_timeout)?,
        cache: TileCache::new(store),
        breaker: CircuitBreaker::new(),
        metrics: Metrics::default(),
        classification: tokio::sync::RwLock::new(None),
        settings: settings.clone(),
    });

    // Die Nutzungsart-Collection ist optional: Ist ALKIS_OSM_PBF_PATH gesetzt,
    // wird die Datei im Hintergrund indiziert (bei einer deutschlandweiten
    // PBF potenziell mehrere Minuten) — der Server wartet nicht darauf,
    // `/health` und die Warnungen der items-Antwort zeigen den Fortschritt.
    // Ein Fehlschlag lässt den Dienst ohne die zweite Collection weiterlaufen,
    // exakt wie ein nicht nutzbarer Cache (siehe oben).
    if let Some(path) = settings.osm_pbf_path.clone() {
        let state_fuer_index = state.clone();
        tokio::spawn(async move {
            tracing::info!(pfad = %path.display(), "Nutzungsart-Index: Indizierung gestartet");
            let ergebnis = {
                let path = path.clone();
                tokio::task::spawn_blocking(move || {
                    alkis_proxy::classification::ClassificationIndex::build(&path)
                })
                .await
            };
            match ergebnis {
                Ok(Ok(index)) => {
                    let features = index.len();
                    *state_fuer_index.classification.write().await = Some(Arc::new(index));
                    tracing::info!(features, "Nutzungsart-Index bereit — Collection flurstuecke-nutzungsart aktiv");
                }
                Ok(Err(e)) => {
                    tracing::warn!(
                        error = %e,
                        pfad = %path.display(),
                        "Nutzungsart-Index konnte nicht gebaut werden — \
                         flurstuecke-nutzungsart bleibt ohne Ergebnisse"
                    );
                }
                Err(e) => {
                    tracing::warn!(error = %e, "Nutzungsart-Indizierung abgebrochen");
                }
            }
        });
    } else {
        tracing::info!("ALKIS_OSM_PBF_PATH nicht gesetzt — Collection flurstuecke-nutzungsart bleibt aus");
    }

    // Kompression spart bei großen FeatureCollections viel Bandbreite, kann
    // aber bei manchen Clients Probleme machen — deshalb abschaltbar.
    let mut app = api::router(state).layer(TraceLayer::new_for_http());
    // Auf Vorhandensein geprüft, nicht auf den Wert — aber ein leerer Wert
    // zählt als nicht gesetzt. Sonst schaltete `ALKIS_DISABLE_COMPRESSION=`
    // aus einer docker-compose.yml die Kompression ungewollt ab, und genau so
    // setzt Compose eine Variable, für die keine .env einen Wert liefert.
    let kompression_aus = std::env::var("ALKIS_DISABLE_COMPRESSION")
        .is_ok_and(|v| !v.trim().is_empty());
    if !kompression_aus {
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
