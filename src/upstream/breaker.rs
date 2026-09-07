//! Circuit Breaker je Bundesland.
//!
//! Der Dienst hängt von 15 fremden Servern ab, von denen einzelne regelmäßig
//! ausfallen — der Saarland-Endpunkt etwa zeigt zeitweise 5xx. Ohne Schutz
//! liefe jede Anfrage in dessen Zeitlimit und risse die Antwortzeit für alle
//! mit. Nach mehreren Fehlschlägen in Folge wird ein Land deshalb für eine
//! Abkühlzeit übersprungen; die Anfrage liefert dann ein Teilergebnis mit
//! Hinweis statt langer Wartezeit.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::config::StateKey;

/// Fehlschläge in Folge, ab denen abgeschaltet wird.
const FAILURE_THRESHOLD: u32 = 5;
/// Wie lange ein Land übersprungen wird.
const COOLDOWN: Duration = Duration::from_secs(60);

#[derive(Debug, Default, Clone, Copy)]
struct Entry {
    consecutive_failures: u32,
    open_until: Option<Instant>,
}

#[derive(Default)]
pub struct CircuitBreaker {
    entries: Mutex<HashMap<StateKey, Entry>>,
}

impl CircuitBreaker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Ob das Land gerade übersprungen werden soll.
    pub fn is_open(&self, state: StateKey) -> bool {
        let mut guard = self.entries.lock().expect("Breaker-Mutex");
        match guard.get_mut(&state).and_then(|e| e.open_until) {
            Some(until) if until > Instant::now() => true,
            Some(_) => {
                // Abkühlzeit vorbei: wieder zulassen, aber den Zähler behalten,
                // damit ein erneuter Fehlschlag sofort wieder öffnet.
                if let Some(e) = guard.get_mut(&state) {
                    e.open_until = None;
                    e.consecutive_failures = FAILURE_THRESHOLD - 1;
                }
                false
            }
            None => false,
        }
    }

    pub fn record_success(&self, state: StateKey) {
        let mut guard = self.entries.lock().expect("Breaker-Mutex");
        guard.insert(state, Entry::default());
    }

    pub fn record_failure(&self, state: StateKey) {
        let mut guard = self.entries.lock().expect("Breaker-Mutex");
        let e = guard.entry(state).or_default();
        e.consecutive_failures += 1;
        if e.consecutive_failures >= FAILURE_THRESHOLD {
            e.open_until = Some(Instant::now() + COOLDOWN);
            tracing::warn!(
                state = state.code(),
                failures = e.consecutive_failures,
                "Landesdienst wird vorübergehend übersprungen"
            );
        }
    }

    /// Momentaufnahme für `/health`: welche Länder gerade abgeschaltet sind.
    pub fn open_states(&self) -> Vec<&'static str> {
        let now = Instant::now();
        let guard = self.entries.lock().expect("Breaker-Mutex");
        let mut v: Vec<_> = guard
            .iter()
            .filter(|(_, e)| e.open_until.is_some_and(|u| u > now))
            .map(|(k, _)| k.code())
            .collect();
        v.sort_unstable();
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oeffnet_erst_nach_mehreren_fehlschlaegen() {
        let b = CircuitBreaker::new();
        for _ in 0..FAILURE_THRESHOLD - 1 {
            b.record_failure(StateKey::Sl);
            assert!(!b.is_open(StateKey::Sl), "zu früh geöffnet");
        }
        b.record_failure(StateKey::Sl);
        assert!(b.is_open(StateKey::Sl));
        assert_eq!(b.open_states(), vec!["SL"]);
    }

    #[test]
    fn erfolg_setzt_den_zaehler_zurueck() {
        let b = CircuitBreaker::new();
        for _ in 0..FAILURE_THRESHOLD - 1 {
            b.record_failure(StateKey::Nw);
        }
        b.record_success(StateKey::Nw);
        b.record_failure(StateKey::Nw);
        assert!(!b.is_open(StateKey::Nw), "Zähler wurde nicht zurückgesetzt");
    }

    #[test]
    fn andere_laender_bleiben_unberuehrt() {
        let b = CircuitBreaker::new();
        for _ in 0..FAILURE_THRESHOLD {
            b.record_failure(StateKey::Sl);
        }
        assert!(b.is_open(StateKey::Sl));
        assert!(!b.is_open(StateKey::Nw));
        assert!(!b.is_open(StateKey::Be));
    }

    #[test]
    fn unbekanntes_land_ist_geschlossen() {
        let b = CircuitBreaker::new();
        assert!(!b.is_open(StateKey::Th));
        assert!(b.open_states().is_empty());
    }
}
