//! Harmonisiert die rohen Nutzungsart-Texte der Landesdienste auf einen
//! einzigen Satz kanonischer Schreibweisen.
//!
//! Recherche (10 Länder live gegen `nutzart`/`objektname` geprüft,
//! 2026-09-10) ergab: Es gibt praktisch **einen** bundesweiten Katalog — der
//! amtliche AdV-Katalog der „tatsächlichen Nutzung". NW, NI, HE, BB, MV, SN,
//! ST und sogar Baden-Württemberg (`objektname`) schreiben ihn
//! buchstabengetreu identisch. Zwei Dienste weichen nur in der
//! **Textkodierung** ab, nicht inhaltlich:
//!
//! * **Thüringen** transliteriert Umlaute/ß (ä→ae, ß→ss …), setzt Title Case
//!   und verschluckt Bindestriche/Kommas/Slashes:
//!   `"Fläche besonderer funktionaler Prägung"` → `"Flaeche Besonderer
//!   Funktionaler Praegung"`.
//! * **Saarland** (ArcGIS) transliteriert nur ß→ss:
//!   `"Straßenverkehr"` → `"Strassenverkehr"`.
//!
//! Ein Falten auf Kleinschreibung + ASCII-Transliteration + Verwerfen aller
//! Satzzeichen macht beide Fälle ununterscheidbar von der kanonischen Form —
//! eine einzige Regel statt einer Sondertabelle je Land.
//!
//! Unbekannte Werte werden **nicht verworfen**: Der Katalog ist aus
//! Stichproben entstanden, nicht aus der amtlichen AAA-Spezifikation
//! abgeschrieben, und dürfte seltene Kategorien (Heide, Moor, Düne, Meer,
//! Hafenbecken, Schiffsverkehr, Bergbaubetrieb …) noch nicht enthalten. Ein
//! unbekannter Wert kommt deshalb roh (nur getrimmt) durch, mit einer
//! Diagnosemeldung — genau wie fehlende Felder anderswo im Adapter-Code nie
//! stillschweigend verschwinden.

use std::collections::HashMap;
use std::sync::LazyLock;

/// Live bestätigte Kategorien des AdV-Katalogs (NW, NI, HE, BB, MV, SN, ST,
/// TH, SL, BW — Stand 2026-09-10). Erweiterbar: ein neuer, bislang
/// unbekannter Wert fällt nicht raus, sondern erscheint unverändert im
/// Ergebnis und in der Log-Ausgabe — von dort lässt er sich hier ergänzen.
const KATALOG: &[&str] = &[
    "Bahnverkehr",
    "Fläche besonderer funktionaler Prägung",
    "Fläche gemischter Nutzung",
    "Fließgewässer",
    "Flugverkehr",
    "Friedhof",
    "Gehölz",
    "Halde",
    "Industrie- und Gewerbefläche",
    "Landwirtschaft",
    "Platz",
    "Sport-, Freizeit- und Erholungsfläche",
    "Stehendes Gewässer",
    "Straßenverkehr",
    "Sumpf",
    "Tagebau/Grube/Steinbruch",
    "Unland/Vegetationslose Fläche",
    "Wald",
    "Weg",
    "Wohnbaufläche",
];

/// Faltschlüssel → kanonische Schreibweise, einmalig aus [`KATALOG`] gebaut.
static NACHSCHLAG: LazyLock<HashMap<String, &'static str>> =
    LazyLock::new(|| KATALOG.iter().map(|&kanon| (fold(kanon), kanon)).collect());

/// Bildet einen rohen Nutzungsart-Text auf seine kanonische Schreibweise ab.
/// Unbekannte Werte kommen getrimmt, aber sonst unverändert durch.
pub fn canonical(raw: &str) -> String {
    let raw = raw.trim();
    match NACHSCHLAG.get(&fold(raw)) {
        Some(&kanon) => kanon.to_string(),
        None => {
            tracing::debug!(wert = raw, "Nutzungsart nicht im Katalog, bleibt roh");
            raw.to_string()
        }
    }
}

/// Kleinschreibung + ASCII-Transliteration der deutschen Sonderzeichen +
/// Verwerfen aller Nicht-Alphanumerik. Macht Thüringens und Saarlands
/// Kodierungsvarianten ununterscheidbar von der kanonischen Form.
fn fold(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.to_lowercase().chars() {
        match c {
            'ä' => out.push_str("ae"),
            'ö' => out.push_str("oe"),
            'ü' => out.push_str("ue"),
            'ß' => out.push_str("ss"),
            c if c.is_alphanumeric() => out.push(c),
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kanonischer_wert_bleibt_unveraendert() {
        assert_eq!(canonical("Straßenverkehr"), "Straßenverkehr");
    }

    #[test]
    fn thueringer_transliteration_wird_erkannt() {
        assert_eq!(
            canonical("Flaeche Besonderer Funktionaler Praegung"),
            "Fläche besonderer funktionaler Prägung"
        );
        assert_eq!(canonical("Strassenverkehr"), "Straßenverkehr");
        assert_eq!(canonical("Industrie Und Gewerbeflaeche"), "Industrie- und Gewerbefläche");
        assert_eq!(
            canonical("Unland Vegetationslose Flaeche"),
            "Unland/Vegetationslose Fläche"
        );
        assert_eq!(canonical("Wohnbauflaeche"), "Wohnbaufläche");
        assert_eq!(canonical("Gehoelz"), "Gehölz");
    }

    #[test]
    fn saarlaendische_transliteration_wird_erkannt() {
        assert_eq!(canonical("Fliessgewässer"), "Fließgewässer");
        assert_eq!(canonical("Strassenverkehr"), "Straßenverkehr");
    }

    #[test]
    fn unbekannter_wert_kommt_roh_aber_getrimmt_durch() {
        assert_eq!(canonical("  Hafenbecken  "), "Hafenbecken");
    }

    #[test]
    fn gross_kleinschreibung_spielt_keine_rolle() {
        assert_eq!(canonical("wohnbaufläche"), "Wohnbaufläche");
        assert_eq!(canonical("WOHNBAUFLÄCHE"), "Wohnbaufläche");
    }
}
