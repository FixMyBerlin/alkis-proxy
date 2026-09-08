//! Laufzeitkonfiguration aus Umgebungsvariablen.
//!
//! Alles über ENV, damit dasselbe Container-Image ohne Änderung auf jedem
//! Server läuft.

use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct Settings {
    /// Adresse, auf der der HTTP-Server lauscht.
    pub bind: String,
    /// Pfad der redb-Cache-Datei. Fehlt er, läuft der Dienst ohne Cache.
    pub cache_path: Option<PathBuf>,
    /// Obergrenze der Cache-Datei in Byte. 0 bedeutet unbegrenzt.
    pub cache_max_bytes: u64,
    /// Zeitlimit für eine einzelne Anfrage an einen Landesdienst.
    pub upstream_timeout: Duration,
    /// Obergrenze für `limit` an der API.
    pub max_limit: usize,
    /// Vorgabe für `limit`, wenn der Client keines angibt.
    pub default_limit: usize,
    /// Nach außen sichtbare Basis-URL. Nur nötig hinter einem Reverse Proxy,
    /// dessen Host-Header von der öffentlichen Adresse abweicht.
    pub public_url: Option<String>,
    /// Pfad einer lokalen OSM-PBF-Datei für die Nutzungsart-Klassifikation.
    /// Fehlt er, bleibt die Collection `flurstuecke-nutzungsart` deaktiviert
    /// und taucht im Katalog nicht auf — analog zu `cache_path`.
    pub osm_pbf_path: Option<PathBuf>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            bind: "0.0.0.0:8080".into(),
            cache_path: None,
            // Sichere Vorgabe, falls ALKIS_CACHE_MAX_SIZE versehentlich
            // fehlt — sonst würde die Cache-Datei unbegrenzt wachsen. Für
            // bundesweite Abdeckung ist das zu knapp bemessen, siehe README.
            cache_max_bytes: 2 * 1024 * 1024 * 1024,
            upstream_timeout: Duration::from_secs(30),
            max_limit: 5_000,
            default_limit: 5_000,
            public_url: None,
            osm_pbf_path: None,
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
            cache_path: env("ALKIS_CACHE_PATH").map(PathBuf::from),
            cache_max_bytes: env("ALKIS_CACHE_MAX_SIZE")
                .and_then(|v| parse_size(&v))
                .unwrap_or(d.cache_max_bytes),
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
            osm_pbf_path: env("ALKIS_OSM_PBF_PATH").map(PathBuf::from),
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

/// Parst Größenangaben wie `32gb`, `512mb` oder eine reine Byte-Zahl.
/// `kb`/`mb`/`gb` rechnen binär (1024er-Basis)
/// `maxmemory` — damit ändert sich für bestehende `.env`-Werte nichts.
fn parse_size(v: &str) -> Option<u64> {
    let v = v.trim().to_lowercase();
    let (digits, factor): (&str, f64) = if let Some(n) = v.strip_suffix("gb") {
        (n, 1024.0 * 1024.0 * 1024.0)
    } else if let Some(n) = v.strip_suffix("mb") {
        (n, 1024.0 * 1024.0)
    } else if let Some(n) = v.strip_suffix("kb") {
        (n, 1024.0)
    } else if let Some(n) = v.strip_suffix('b') {
        (n, 1.0)
    } else {
        (v.as_str(), 1.0)
    };
    let n: f64 = digits.trim().parse().ok()?;
    if n < 0.0 {
        return None;
    }
    Some((n * factor) as u64)
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
        assert_eq!(s.clamp_limit(Some(999_999)), 5_000);
    }

    #[test]
    fn vorgaben_sind_plausibel() {
        let s = Settings::default();
        assert!(s.default_limit <= s.max_limit);
        assert!(s.cache_path.is_none(), "ohne ENV kein Cache");
    }

    #[test]
    fn groessenangaben_werden_binaer_geparst() {
        assert_eq!(parse_size("512mb"), Some(512 * 1024 * 1024));
        assert_eq!(parse_size("32gb"), Some(32 * 1024 * 1024 * 1024));
        assert_eq!(parse_size("1kb"), Some(1024));
        assert_eq!(parse_size("100"), Some(100));
        assert_eq!(parse_size("  2GB  "), Some(2 * 1024 * 1024 * 1024));
        assert_eq!(parse_size("nonsense"), None);
    }
}
