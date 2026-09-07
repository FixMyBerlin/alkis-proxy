//! HTTP-Zugriff auf die Landesdienste.
//!
//! Ein gemeinsamer [`reqwest::Client`] hält Verbindungen pro Host offen. Das ist
//! spürbar: Eine Anfrage löst oft mehrere Kachelabrufe gegen denselben Dienst
//! aus, und ohne Keep-Alive käme für jeden ein TLS-Handshake dazu.

use std::time::Duration;

use crate::adapters;
use crate::config::StateConfig;
use crate::crs::reproject::{geometry_to_wgs84, Bbox};
use crate::model::Parcel;
use crate::parse::{parse_geojson, parse_gml};
use crate::upstream::fetch::{build_get_feature_url, UpstreamError};

/// Ergebnis eines Kachelabrufs.
#[derive(Debug)]
pub struct TileResult {
    pub parcels: Vec<Parcel>,
    /// Der Dienst hat genau so viele Features geliefert, wie angefordert waren —
    /// die Antwort ist damit vermutlich abgeschnitten und die Kachel muss
    /// geteilt werden.
    pub truncated: bool,
}

pub struct UpstreamClient {
    http: reqwest::Client,
    timeout: Duration,
}

impl UpstreamClient {
    pub fn new(timeout: Duration) -> Result<Self, reqwest::Error> {
        let http = reqwest::Client::builder()
            .timeout(timeout)
            .pool_max_idle_per_host(8)
            .pool_idle_timeout(Duration::from_secs(90))
            .user_agent(concat!("alkis-proxy/", env!("CARGO_PKG_VERSION")))
            .build()?;
        Ok(UpstreamClient { http, timeout })
    }

    /// Holt alle Flurstücke innerhalb der BBOX (im nativen CRS des Landes) und
    /// gibt sie in EPSG:4326 zurück.
    pub async fn fetch(
        &self,
        state: &StateConfig,
        bbox: Bbox,
        count: usize,
    ) -> Result<TileResult, UpstreamError> {
        let code = state.key.code();
        let endpoint = state.endpoint.ok_or(UpstreamError::NoEndpoint { state: code })?;

        let url = build_get_feature_url(&endpoint, bbox, count);
        let response = self.http.get(&url).send().await.map_err(|e| {
            if e.is_timeout() {
                UpstreamError::Timeout {
                    state: code,
                    seconds: self.timeout.as_secs(),
                }
            } else {
                UpstreamError::Network {
                    state: code,
                    message: e.to_string(),
                }
            }
        })?;

        if !response.status().is_success() {
            return Err(UpstreamError::HttpStatus {
                state: code,
                status: response.status().as_u16(),
            });
        }

        let body = response.text().await.map_err(|e| UpstreamError::Network {
            state: code,
            message: e.to_string(),
        })?;

        // Das Format wird am Inhalt erkannt, nicht an der Konfiguration: Dienste
        // liefern auch mal etwas anderes als angefordert (Fehler als XML trotz
        // JSON-Anfrage), und bei ServerDefault ist es ohnehin offen.
        // Fehler kommen dabei häufig mit HTTP 200 als OGC-Exception; die Parser
        // erkennen das und melden es hier weiter.
        let raws = if body.trim_start().starts_with('<') {
            parse_gml(&body).map_err(|e| UpstreamError::Unparsable {
                state: code,
                message: e.to_string(),
            })?
        } else {
            parse_geojson(&body).map_err(|e| UpstreamError::Unparsable {
                state: code,
                message: e.to_string(),
            })?
        };

        let truncated = raws.len() >= count;

        let mut parcels = Vec::with_capacity(raws.len());
        for raw in &raws {
            match adapters::map(endpoint.schema, raw, state.key) {
                Ok(mut parcel) => {
                    // Ab hier ist alles WGS84 — auch im Cache, damit ein Treffer
                    // ohne weitere Rechenarbeit ausgeliefert werden kann.
                    geometry_to_wgs84(&mut parcel.geometry, endpoint.native_crs);
                    parcels.push(parcel);
                }
                Err(e) => {
                    // Ein einzelnes unbrauchbares Feature darf die Kachel nicht
                    // scheitern lassen — das passiert bei Schema-Drift, und ein
                    // Teilergebnis ist besser als gar keins.
                    tracing::warn!(state = code, error = %e, "Feature übersprungen");
                }
            }
        }

        Ok(TileResult { parcels, truncated })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_laesst_sich_bauen() {
        assert!(UpstreamClient::new(Duration::from_secs(30)).is_ok());
    }

    #[test]
    fn truncation_wird_an_der_grenze_erkannt() {
        // Die Logik selbst: genau count Features gelten als abgeschnitten.
        for (returned, count, expected) in [(0, 10, false), (9, 10, false), (10, 10, true)] {
            assert_eq!(returned >= count, expected, "{returned} von {count}");
        }
    }
}
