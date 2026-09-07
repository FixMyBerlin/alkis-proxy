//! Zähler für `/metrics` im Prometheus-Textformat.
//!
//! Bewusst handgeschrieben statt über eine Bibliothek: Es sind eine Handvoll
//! Zähler, und der Dienst bleibt ohne weitere Abhängigkeit.

use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Default)]
pub struct Metrics {
    pub requests: AtomicU64,
    pub cache_hits: AtomicU64,
    pub cache_misses: AtomicU64,
    pub upstream_errors: AtomicU64,
    pub features_served: AtomicU64,
}

impl Metrics {
    pub fn add(counter: &AtomicU64, n: u64) {
        counter.fetch_add(n, Ordering::Relaxed);
    }

    /// Prometheus-Textformat.
    pub fn render(&self, open_states: &[&str], cache_enabled: bool) -> String {
        let g = |c: &AtomicU64| c.load(Ordering::Relaxed);
        let mut s = String::new();
        for (name, help, value) in [
            ("alkis_requests_total", "Beantwortete API-Anfragen", g(&self.requests)),
            ("alkis_cache_hits_total", "Kacheln aus dem Cache", g(&self.cache_hits)),
            ("alkis_cache_misses_total", "Kacheln vom Landesdienst", g(&self.cache_misses)),
            ("alkis_upstream_errors_total", "Fehlgeschlagene Abrufe", g(&self.upstream_errors)),
            ("alkis_features_served_total", "Ausgelieferte Flurstücke", g(&self.features_served)),
        ] {
            s.push_str(&format!("# HELP {name} {help}\n# TYPE {name} counter\n{name} {value}\n"));
        }
        s.push_str("# HELP alkis_cache_enabled Ob ein Cache angebunden ist\n");
        s.push_str("# TYPE alkis_cache_enabled gauge\n");
        s.push_str(&format!("alkis_cache_enabled {}\n", u8::from(cache_enabled)));
        s.push_str("# HELP alkis_breaker_open Landesdienst wird derzeit übersprungen\n");
        s.push_str("# TYPE alkis_breaker_open gauge\n");
        for code in open_states {
            s.push_str(&format!("alkis_breaker_open{{state=\"{code}\"}} 1\n"));
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rendert_prometheus_format() {
        let m = Metrics::default();
        Metrics::add(&m.requests, 3);
        Metrics::add(&m.cache_hits, 7);
        let out = m.render(&["SL"], true);
        assert!(out.contains("alkis_requests_total 3"), "{out}");
        assert!(out.contains("alkis_cache_hits_total 7"), "{out}");
        assert!(out.contains("alkis_cache_enabled 1"), "{out}");
        assert!(out.contains(r#"alkis_breaker_open{state="SL"} 1"#), "{out}");
        // Jede Metrik braucht HELP und TYPE, sonst verwirft Prometheus sie.
        assert_eq!(out.matches("# HELP").count(), out.matches("# TYPE").count());
    }
}
