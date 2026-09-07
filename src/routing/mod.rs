//! Zuordnung einer Anfrage zu den zuständigen Bundesländern.
//!
//! Der Dienst muss wissen, welchen Landes-WFS er für einen Kartenausschnitt
//! fragen soll. Statt exakter Grenzpolygone genügt dafür je Bundesland eine
//! Hüllbox in WGS84:
//!
//! * Eine Anfrage, die mehrere Boxen schneidet, geht an alle betroffenen Länder —
//!   nötig ist das ohnehin, sobald ein Ausschnitt über eine Landesgrenze reicht.
//! * Ein zu Unrecht befragtes Land liefert für den Ausschnitt schlicht keine
//!   Features. Das Verfahren ist damit selbstkorrigierend; zu großzügige Boxen
//!   kosten Latenz, aber erzeugen keine falschen Ergebnisse.
//!
//! Exakte Polygone (BKG VG250) würden die überflüssigen Abfragen an den
//! Grenzen einsparen. Das lohnt sich, sobald die Latenz dort stört — die
//! Schnittstelle dieses Moduls bleibt davon unberührt.

use crate::config::{states, StateConfig, StateKey};
use crate::crs::reproject::Bbox;

/// Hüllbox eines Bundeslands in WGS84: `(lon_min, lat_min, lon_max, lat_max)`.
const EXTENTS: &[(StateKey, f64, f64, f64, f64)] = &[
    (StateKey::Sh, 7.87, 53.36, 11.31, 55.06),
    (StateKey::Hh, 8.42, 53.39, 10.33, 53.97),
    (StateKey::Ni, 6.65, 51.29, 11.60, 53.89),
    (StateKey::Hb, 8.48, 53.01, 8.99, 53.61),
    (StateKey::Nw, 5.87, 50.32, 9.46, 52.53),
    (StateKey::He, 7.77, 49.40, 10.24, 51.65),
    (StateKey::Rp, 6.11, 48.97, 8.51, 50.94),
    (StateKey::Bw, 7.51, 47.53, 10.50, 49.79),
    (StateKey::By, 8.98, 47.27, 13.84, 50.56),
    (StateKey::Sl, 6.36, 49.11, 7.40, 49.64),
    (StateKey::Be, 13.09, 52.34, 13.76, 52.68),
    (StateKey::Bb, 11.27, 51.36, 14.77, 53.56),
    (StateKey::Mv, 10.59, 53.11, 14.41, 54.68),
    (StateKey::Sn, 11.87, 50.17, 15.04, 51.69),
    (StateKey::St, 10.56, 50.94, 13.19, 53.04),
    (StateKey::Th, 9.88, 50.20, 12.65, 51.65),
];

/// Alle Bundesländer, deren Ausdehnung die BBOX (WGS84) schneidet und für die
/// ein Endpunkt konfiguriert ist.
pub fn states_for_bbox(bbox: Bbox) -> Vec<&'static StateConfig> {
    EXTENTS
        .iter()
        .filter(|(_, w, s, e, n)| {
            // Rechteckschnitt: keine Trennung in einer der beiden Achsen.
            bbox.min_x <= *e && bbox.max_x >= *w && bbox.min_y <= *n && bbox.max_y >= *s
        })
        .map(|(key, ..)| states::get(*key))
        .filter(|s| s.endpoint.is_some())
        .collect()
}

/// Das Bundesland, in dem ein Punkt liegt — bei Überlappung mehrere.
pub fn states_for_point(lon: f64, lat: f64) -> Vec<&'static StateConfig> {
    states_for_bbox(Bbox::new(lon, lat, lon, lat))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn codes(v: Vec<&StateConfig>) -> Vec<&'static str> {
        let mut c: Vec<_> = v.iter().map(|s| s.key.code()).collect();
        c.sort_unstable();
        c
    }

    #[test]
    fn koeln_wird_nordrhein_westfalen_zugeordnet() {
        // Köln liegt auf der Nordkante der Hüllbox von Rheinland-Pfalz, das
        // deshalb mitgefragt wird. Erwartet wird nicht Exklusivität, sondern
        // dass das zuständige Land dabei ist — ein Nachbar ohne Daten im
        // Ausschnitt liefert nichts und verfälscht das Ergebnis nicht.
        let c = codes(states_for_point(6.958, 50.940));
        assert!(c.contains(&"NW"), "{c:?}");
        assert!(c.len() <= 3, "zu viele Kandidaten: {c:?}");
    }

    #[test]
    fn stuttgart_liegt_in_baden_wuerttemberg() {
        assert_eq!(codes(states_for_point(9.178, 48.776)), vec!["BW"]);
    }

    #[test]
    fn berlin_liefert_auch_brandenburg() {
        // Berlin liegt vollständig in der Hüllbox Brandenburgs. Beide zu fragen
        // ist korrekt: An der Stadtgrenze reichen Ausschnitte über beide Länder.
        let c = codes(states_for_point(13.400, 52.510));
        assert!(c.contains(&"BE"), "{c:?}");
        assert!(c.contains(&"BB"), "{c:?}");
    }

    #[test]
    fn bayern_hat_keinen_endpunkt_und_faellt_heraus() {
        // München liegt in Bayern, für das kein Dienst konfiguriert ist.
        let c = codes(states_for_point(11.576, 48.137));
        assert!(!c.contains(&"BY"), "{c:?}");
    }

    #[test]
    fn ausschnitt_ueber_die_landesgrenze_fragt_beide() {
        // Zwischen Nordrhein-Westfalen und Niedersachsen.
        let c = codes(states_for_bbox(Bbox::new(7.90, 52.20, 8.10, 52.40)));
        assert!(c.contains(&"NW"), "{c:?}");
        assert!(c.contains(&"NI"), "{c:?}");
    }

    #[test]
    fn ausserhalb_deutschlands_liefert_nichts() {
        assert!(states_for_point(2.35, 48.86).is_empty(), "Paris");
        assert!(states_for_point(16.37, 48.21).is_empty(), "Wien");
    }

    #[test]
    fn jedes_bundesland_hat_eine_ausdehnung() {
        assert_eq!(EXTENTS.len(), 16);
        for (key, w, s, e, n) in EXTENTS {
            assert!(w < e && s < n, "{}: Box verdreht", key.code());
            assert!((5.0..16.0).contains(w), "{}: außerhalb DE", key.code());
            assert!((47.0..56.0).contains(s), "{}: außerhalb DE", key.code());
        }
    }
}
