//! Fall 1: Länder mit vorverschnittener `NutzungFlurstueck`-Ebene.
//!
//! Acht Länder (NW, NI, HE, BB, MV, SN, HH, SL) führen neben `ave:Flurstueck`
//! eine zweite Ebene, die bereits mit dem Flurstück verschnitten ist: jede
//! Zeile trägt `flstkennz` und `nutzart`, ein Flurstück mit mehreren
//! tatsächlichen Nutzungen erscheint mehrfach. Stichprobe NRW (200 Flurstücke
//! aus einem Stadtrand-Ausschnitt, 2026-09-10): 244 zugeordnete Flurstücke,
//! davon 105 mit mehr als einer Nutzung. Keine Geometriearbeit nötig — reiner
//! Attribut-Join über das Kennzeichen.

use std::collections::HashMap;

use crate::nutzung::catalog;
use crate::parse::RawFeature;

/// Felder, unter denen das Flurstückskennzeichen auftaucht — dieselbe Liste
/// wie beim Flurstück selbst (`adapters::ave`), damit der Join unabhängig
/// davon greift, welchen Namen ein Land für das Feld wählt.
const KENNZEICHEN_FELDER: &[&str] = &["flstkennz", "flurstueckskennzeichen"];

/// Nachschlagetabelle Flurstückskennzeichen → semikolongetrennte, deduplizierte
/// Nutzungsarten.
pub struct JoinIndex(HashMap<String, String>);

impl JoinIndex {
    /// Baut den Index aus den Rohfeatures der `NutzungFlurstueck`-Antwort.
    ///
    /// Reihenfolge bleibt Erscheinungsreihenfolge in der Antwort — ohne
    /// Sortierung, weil die Dienste selbst keine stabile Ordnung zusagen und
    /// eine zusätzliche Sortierung hier keinen Wert hätte, den ein Client
    /// nicht ebenso gut selbst herstellen könnte.
    pub fn build(raws: &[RawFeature]) -> Self {
        let mut by_kennzeichen: HashMap<String, Vec<String>> = HashMap::new();
        for raw in raws {
            let Some(kennzeichen) = raw.field(KENNZEICHEN_FELDER) else { continue };
            let Some(nutzart) = raw.field(&["nutzart"]) else { continue };
            let nutzart = catalog::canonical(nutzart);
            let arten = by_kennzeichen.entry(kennzeichen.to_string()).or_default();
            if !arten.contains(&nutzart) {
                arten.push(nutzart);
            }
        }
        JoinIndex(
            by_kennzeichen
                .into_iter()
                .map(|(kennzeichen, arten)| (kennzeichen, arten.join(";")))
                .collect(),
        )
    }

    /// `nutzungALKIS` für das Flurstück, dessen Rohfeature übergeben wird.
    pub fn lookup(&self, raw: &RawFeature) -> Option<String> {
        let kennzeichen = raw.field(KENNZEICHEN_FELDER)?;
        self.0.get(kennzeichen).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nutzung_flurstueck(kennzeichen: &str, nutzart: &str) -> RawFeature {
        let mut raw = RawFeature::default();
        raw.fields.insert("flstkennz".into(), kennzeichen.into());
        raw.fields.insert("nutzart".into(), nutzart.into());
        raw
    }

    fn flurstueck(kennzeichen: &str) -> RawFeature {
        let mut raw = RawFeature::default();
        raw.fields.insert("flstkennz".into(), kennzeichen.into());
        raw
    }

    #[test]
    fn einzelne_nutzung_wird_gefunden() {
        let raws = vec![nutzung_flurstueck("K1", "Straßenverkehr")];
        let index = JoinIndex::build(&raws);
        assert_eq!(index.lookup(&flurstueck("K1")).as_deref(), Some("Straßenverkehr"));
    }

    #[test]
    fn mehrere_nutzungen_werden_semikolongetrennt() {
        let raws = vec![
            nutzung_flurstueck("K1", "Landwirtschaft"),
            nutzung_flurstueck("K1", "Weg"),
            nutzung_flurstueck("K1", "Gehölz"),
        ];
        let index = JoinIndex::build(&raws);
        assert_eq!(
            index.lookup(&flurstueck("K1")).as_deref(),
            Some("Landwirtschaft;Weg;Gehölz")
        );
    }

    #[test]
    fn doppelte_nutzart_wird_dedupliziert() {
        // Wie im echten NRW-Sample: Fließgewässer taucht zweimal auf, weil
        // der Bach das Flurstück zweimal quert.
        let raws = vec![
            nutzung_flurstueck("K1", "Fließgewässer"),
            nutzung_flurstueck("K1", "Landwirtschaft"),
            nutzung_flurstueck("K1", "Fließgewässer"),
        ];
        let index = JoinIndex::build(&raws);
        assert_eq!(
            index.lookup(&flurstueck("K1")).as_deref(),
            Some("Fließgewässer;Landwirtschaft")
        );
    }

    #[test]
    fn flurstueck_ohne_treffer_bleibt_unbelegt() {
        let raws = vec![nutzung_flurstueck("K1", "Weg")];
        let index = JoinIndex::build(&raws);
        assert!(index.lookup(&flurstueck("K2")).is_none());
    }

    #[test]
    fn alternativer_kennzeichenname_wird_erkannt() {
        let mut raw = RawFeature::default();
        raw.fields.insert("flurstueckskennzeichen".into(), "K1".into());
        raw.fields.insert("nutzart".into(), "Wald".into());
        let index = JoinIndex::build(&[raw]);
        assert_eq!(index.lookup(&flurstueck("K1")).as_deref(), Some("Wald"));
    }

    #[test]
    fn leere_antwort_liefert_nichts() {
        let index = JoinIndex::build(&[]);
        assert!(index.lookup(&flurstueck("K1")).is_none());
    }
}
