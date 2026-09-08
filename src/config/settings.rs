//! Laufzeitkonfiguration aus Umgebungsvariablen.
//!
//! Alles über ENV, damit dasselbe Container-Image ohne Änderung auf jedem
//! Server läuft.

use std::time::Duration;

#[derive(Debug, Clone)]
pub struct Settings {
    /// Adresse, auf der der HTTP-Server lauscht.
    pub bind: String,
    /// Valkey-URL. Fehlt sie, läuft der Dienst ohne Cache.
    pub valkey_url: Option<String>,
    /// Zeitlimit für eine einzelne Anfrage an einen Landesdienst.
    pub upstream_timeout: Duration,
    /// Obergrenze für `limit` an der API.
    pub max_limit: usize,
    /// Vorgabe für `limit`, wenn der Client keines angibt.
    pub default_limit: usize,
    /// Nach außen sichtbare Basis-URL. Nur nötig hinter einem Reverse Proxy,
    /// dessen Host-Header von der öffentlichen Adresse abweicht.
    pub public_url: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            bind: "0.0.0.0:8080".into(),
            valkey_url: None,
            upstream_timeout: Duration::from_secs(30),
            max_limit: 10_000,
            default_limit: 5_000,
            public_url: None,
        }
    }
}

impl Settings {
    /// Liest die Konfiguration aus der Umgebung; nicht gesetzte Werte behalten
    /// ihre Vorgabe.
    pub fn from_env() -> Self {
        let d = Settings::default();
        Settings {
            bind: env("ALKIS_BIND").unwrap_or(d.bind),
            valkey_url: env("ALKIS_VALKEY_URL"),
            upstream_timeout: env("ALKIS_UPSTREAM_TIMEOUT_SECS")
                .and_then(|v| v.parse().ok())
                .map(Duration::from_secs)
                .unwrap_or(d.upstream_timeout),
            max_limit: env("ALKIS_MAX_LIMIT")
                .and_then(|v| v.parse().ok())
                .unwrap_or(d.max_limit),
            default_limit: env("ALKIS_DEFAULT_LIMIT")
                .and_then(|v| v.parse().ok())
                .unwrap_or(d.default_limit),
            public_url: env("ALKIS_PUBLIC_URL"),
        }
    }

    /// Begrenzt ein vom Client gewünschtes Limit auf den erlaubten Bereich.
    pub fn clamp_limit(&self, requested: Option<usize>) -> usize {
        requested.unwrap_or(self.default_limit).clamp(1, self.max_limit)
    }
}

fn env(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limit_wird_begrenzt() {
        let s = Settings::default();
        assert_eq!(s.clamp_limit(None), 5_000);
        assert_eq!(s.clamp_limit(Some(100)), 100);
        assert_eq!(s.clamp_limit(Some(0)), 1, "0 ergibt keine sinnvolle Antwort");
        assert_eq!(s.clamp_limit(Some(999_999)), 10_000);
    }

    #[test]
    fn vorgaben_sind_plausibel() {
        let s = Settings::default();
        assert!(s.default_limit <= s.max_limit);
        assert!(s.valkey_url.is_none(), "ohne ENV kein Cache");
    }
}
