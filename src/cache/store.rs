//! Valkey-Anbindung.
//!
//! Der Cache ist bewusst *optional*: Ist Valkey nicht erreichbar, arbeitet der
//! Dienst ohne ihn weiter — langsamer, aber korrekt. Ein Cache-Ausfall darf
//! nicht zum Dienstausfall werden.
//!
//! Gespeichert werden zstd-komprimierte GeoJSON-Feature-Listen. Komprimieren
//! lohnt sich hier deutlich: Flurstücksgeometrien sind lange Zahlenreihen mit
//! viel Wiederholung, typisch bleiben unter 20 % der Rohgröße übrig.

use std::time::Duration;

use deadpool_redis::{Config, Pool, Runtime};
use redis::AsyncCommands;

/// Kompressionsstufe. 3 ist der zstd-Standard: nahe am besten Verhältnis,
/// aber um ein Vielfaches schneller als die hohen Stufen.
const ZSTD_LEVEL: i32 = 3;

pub struct ValkeyStore {
    pool: Pool,
}

impl ValkeyStore {
    /// Baut den Verbindungspool auf und prüft ihn mit einem PING.
    ///
    /// Schlägt das fehl, gibt der Aufrufer `None` weiter und der Dienst läuft
    /// ohne Cache.
    pub async fn connect(url: &str) -> Result<Self, String> {
        let cfg = Config::from_url(url);
        let pool = cfg
            .create_pool(Some(Runtime::Tokio1))
            .map_err(|e| format!("Valkey-Pool: {e}"))?;

        let mut conn = pool
            .get()
            .await
            .map_err(|e| format!("Valkey nicht erreichbar: {e}"))?;
        redis::cmd("PING")
            .query_async::<String>(&mut conn)
            .await
            .map_err(|e| format!("Valkey antwortet nicht: {e}"))?;

        Ok(ValkeyStore { pool })
    }

    /// Liest und entpackt einen Eintrag. Fehler werden zu `None` — ein kaputter
    /// Cache-Eintrag soll die Anfrage nicht scheitern lassen, sondern nur einen
    /// Neuabruf auslösen.
    pub async fn get(&self, key: &str) -> Option<Vec<u8>> {
        let mut conn = self.pool.get().await.ok()?;
        let packed: Vec<u8> = conn.get(key).await.ok()?;
        if packed.is_empty() {
            return None;
        }
        match zstd::decode_all(packed.as_slice()) {
            Ok(raw) => Some(raw),
            Err(e) => {
                tracing::warn!(key, error = %e, "Cache-Eintrag nicht entpackbar");
                None
            }
        }
    }

    /// Schreibt einen Eintrag mit Ablauffrist. Fehler werden nur geloggt.
    pub async fn set(&self, key: &str, value: &[u8], ttl: Duration) {
        let packed = match zstd::encode_all(value, ZSTD_LEVEL) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(key, error = %e, "Cache-Eintrag nicht komprimierbar");
                return;
            }
        };
        let Ok(mut conn) = self.pool.get().await else {
            return;
        };
        let result: redis::RedisResult<()> = conn.set_ex(key, packed, ttl.as_secs()).await;
        if let Err(e) = result {
            tracing::warn!(key, error = %e, "Cache-Eintrag nicht speicherbar");
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn zstd_komprimiert_geojson_deutlich() {
        // Repräsentativer Ausschnitt: Koordinatenlisten mit viel Wiederholung.
        let json = r#"{"type":"Feature","geometry":{"type":"MultiPolygon","coordinates":
            [[[[356447.097,5644891.501],[356447.475,5644891.435],[356448.128,5644909.273],
            [356448.427,5644909.241],[356449.614,5644923.488],[356449.300,5644923.376]]]]}}"#
            .repeat(20);
        let packed = zstd::encode_all(json.as_bytes(), super::ZSTD_LEVEL).unwrap();
        let quote = packed.len() as f64 / json.len() as f64;
        assert!(quote < 0.25, "nur auf {:.0} % komprimiert", quote * 100.0);
        let back = zstd::decode_all(packed.as_slice()).unwrap();
        assert_eq!(back, json.as_bytes());
    }
}
