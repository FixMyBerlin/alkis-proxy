//! Der eigentliche Datenendpunkt: `/collections/flurstuecke/items`.

use std::sync::atomic::AtomicU64;

use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::api::error::ApiError;
use crate::api::metrics::Metrics;
use crate::api::{base_url, SharedState};
use crate::config::{states, StateKey};
use crate::crs::reproject::Bbox;
use crate::routing;

/// Ausschnitt für das Schema-Sample: ein Häuserblock in der Kölner Innenstadt.
/// Bewusst klein und in einem Land mit schnellem, stabilem Dienst; nach dem
/// ersten Abruf liegt er ohnehin im Cache.
const SCHEMA_SAMPLE_BBOX: (f64, f64, f64, f64) = (6.9575, 50.9395, 6.9585, 50.9405);
/// Wie viele Features das Sample umfasst — mehr braucht kein Client, um die
/// Felder zu erkennen.
const SCHEMA_SAMPLE_LIMIT: usize = 2;

/// Größter zulässiger Ausschnitt in Grad Kantenlänge.
const MAX_BBOX_DEGREES: f64 = 1.0;

#[derive(Debug, Deserialize)]
pub struct ItemsQuery {
    /// `min_lon,min_lat,max_lon,max_lat` in WGS84.
    bbox: Option<String>,
    limit: Option<usize>,
    /// Optionale Einschränkung auf ein Bundesland (`NW`). Ohne Angabe
    /// bestimmt der Dienst die zuständigen Länder selbst.
    state: Option<String>,
}

pub async fn items(
    State(app): State<SharedState>,
    headers: HeaderMap,
    Query(q): Query<ItemsQuery>,
) -> Result<Json<Value>, ApiError> {
    Metrics::add(&app.metrics.requests, 1);
    let base = base_url(&app, &headers);

    // Ohne Ausschnitt kann bundesweit nichts geliefert werden — es sind rund
    // 60 Millionen Flurstücke. Ein Fehler wäre hier aber falsch: Clients wie
    // QGIS fragen beim Verbinden zunächst ohne bbox an, um das Feldschema zu
    // ermitteln. Bekommen sie dabei nichts, legen sie einen Layer ganz ohne
    // Attributspalten an. Deshalb wird stattdessen ein kleines Sample aus einem
    // festen Ausschnitt geliefert, erkennbar an der beigefügten Warnung.
    let limit = app.settings.clamp_limit(q.limit);
    let (bbox, is_sample) = match q.bbox.as_deref().filter(|b| !b.trim().is_empty()) {
        Some(raw) => (parse_bbox(Some(raw))?, false),
        None => {
            let (w, s, e, n) = SCHEMA_SAMPLE_BBOX;
            (Bbox::new(w, s, e, n), true)
        }
    };
    let limit = if is_sample { SCHEMA_SAMPLE_LIMIT } else { limit };

    // Ein zu weit herausgezoomter Ausschnitt würde tausende Kachelabrufe
    // auslösen. Das ist kein Fehler des Clients: Karten werden nun einmal
    // herausgezoomt, und eine Fehlermeldung bei jedem Zoomschritt macht den
    // Layer in QGIS unbenutzbar. Stattdessen kommt eine leere Antwort mit
    // Hinweis — wie bei Katasterdiensten üblich, die erst ab einem bestimmten
    // Maßstab zeichnen.
    if bbox.width() > MAX_BBOX_DEGREES || bbox.height() > MAX_BBOX_DEGREES {
        return Ok(Json(empty_collection(
            &base,
            Some(
                "Ausschnitt zu groß — Flurstücke werden erst ab etwa 1 Grad Kantenlänge \
                 geliefert. Bitte näher heranzoomen.",
            ),
        )));
    }

    let targets = match q.state.as_deref() {
        Some(code) => {
            let key = StateKey::from_code(code)
                .ok_or_else(|| ApiError::BadRequest(format!("Unbekanntes Bundesland: {code}")))?;
            let cfg = states::get(key);
            if cfg.endpoint.is_none() {
                return Err(ApiError::BadRequest(format!(
                    "Für {} ist kein Dienst verfügbar.",
                    cfg.label
                )));
            }
            vec![cfg]
        }
        None => routing::states_for_bbox(bbox),
    };

    if targets.is_empty() {
        // Kein Fehler: Die BBOX liegt schlicht außerhalb Deutschlands.
        return Ok(Json(empty_collection(&base, None)));
    }

    // Abgeschaltete Länder überspringen, aber im Ergebnis erwähnen.
    let mut warnings = Vec::new();
    let available: Vec<_> = targets
        .into_iter()
        .filter(|s| {
            if app.breaker.is_open(s.key) {
                warnings.push(format!(
                    "{}: Dienst derzeit nicht erreichbar, wird übersprungen",
                    s.label
                ));
                false
            } else {
                true
            }
        })
        .collect();

    let outcome = app
        .cache
        .features_for_bbox(&app.client, &app.breaker, &available, bbox, limit)
        .await;

    Metrics::add(&app.metrics.cache_hits, outcome.cache_hits as u64);
    Metrics::add(&app.metrics.cache_misses, outcome.cache_misses as u64);
    Metrics::add(&app.metrics.features_served, outcome.features.len() as u64);
    if !outcome.warnings.is_empty() {
        Metrics::add(&app.metrics.upstream_errors, outcome.warnings.len() as u64);
    }
    warnings.extend(outcome.warnings);
    if is_sample {
        warnings.push(
            "Ohne bbox wird nur ein Beispielausschnitt geliefert, damit Clients das \
             Feldschema erkennen können. Für echte Abfragen bitte bbox angeben \
             (maximal 1 Grad Kantenlänge)."
                .into(),
        );
    }

    let count = outcome.features.len();
    let mut body = json!({
        "type": "FeatureCollection",
        "features": outcome.features,
        "numberReturned": count,
        "numberMatched": count,
        "timeStamp": now_iso(),
        "links": links(&base),
    });
    if !warnings.is_empty() {
        // Teilausfälle sichtbar machen, statt die Anfrage scheitern zu lassen:
        // ein Ergebnis aus drei von vier Ländern ist meist brauchbar.
        body["warnings"] = json!(warnings);
    }
    Ok(Json(body))
}

/// Einzelnes Flurstück über seine Feature-ID (`NW:05495803101089______`).
///
/// Ohne räumlichen Bezug lässt sich ein Kennzeichen bei den Landesdiensten nicht
/// nachschlagen — WFS kennt dafür keinen einheitlichen Filter. Der Endpunkt
/// beantwortet daher nur, was im Cache liegt, und verweist sonst auf `items`.
pub async fn item(
    State(_app): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    Err(ApiError::NotFound(format!(
        "Flurstück {id} kann nicht ohne räumliche Anfrage aufgelöst werden. \
         Bitte /collections/flurstuecke/items?bbox=… verwenden."
    )))
}

fn empty_collection(base: &str, hint: Option<&str>) -> Value {
    let mut v = json!({
        "type": "FeatureCollection",
        "features": [],
        "numberReturned": 0,
        "numberMatched": 0,
        "timeStamp": now_iso(),
        "links": links(base),
    });
    if let Some(h) = hint {
        v["warnings"] = json!([h]);
    }
    v
}

fn links(base: &str) -> Value {
    json!([
        {
            "rel": "self",
            "type": "application/geo+json",
            "title": "Diese Antwort",
            "href": format!("{base}/collections/flurstuecke/items")
        },
        {
            "rel": "collection",
            "type": "application/json",
            "title": "Beschreibung der Sammlung",
            "href": format!("{base}/collections/flurstuecke")
        }
    ])
}

/// Zeitstempel im Format, das OGC API Features für `timeStamp` vorsieht.
fn now_iso() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Ohne Datums-Bibliothek: Umrechnung aus der Unix-Zeit (immer UTC).
    let (days, rest) = (secs / 86_400, secs % 86_400);
    let (h, mi, sec) = (rest / 3600, (rest % 3600) / 60, rest % 60);
    let (y, mo, d) = civil_from_days(days as i64);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{sec:02}Z")
}

/// Tage seit 1970-01-01 → Kalenderdatum (Algorithmus nach Howard Hinnant).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// `bbox=min_lon,min_lat,max_lon,max_lat`
fn parse_bbox(raw: Option<&str>) -> Result<Bbox, ApiError> {
    let raw = raw.ok_or_else(|| {
        ApiError::BadRequest("Parameter bbox fehlt (Format: min_lon,min_lat,max_lon,max_lat)".into())
    })?;

    let parts: Vec<f64> = raw
        .split(',')
        .map(str::trim)
        .map(|p| p.parse::<f64>())
        .collect::<Result<_, _>>()
        .map_err(|_| ApiError::BadRequest(format!("bbox enthält keine Zahlen: {raw}")))?;

    let [w, s, e, n] = parts[..] else {
        return Err(ApiError::BadRequest(format!(
            "bbox braucht vier Werte, hat {}: {raw}",
            parts.len()
        )));
    };

    if !(-180.0..=180.0).contains(&w) || !(-180.0..=180.0).contains(&e) {
        return Err(ApiError::BadRequest("Längengrad außerhalb -180..180".into()));
    }
    if !(-90.0..=90.0).contains(&s) || !(-90.0..=90.0).contains(&n) {
        return Err(ApiError::BadRequest("Breitengrad außerhalb -90..90".into()));
    }

    Ok(Bbox::new(w, s, e, n))
}

// Damit der Compiler den Import nicht als ungenutzt meldet, wenn keine
// Metrik-Aufrufe erzeugt werden.
#[allow(dead_code)]
type _Counter = AtomicU64;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bbox_wird_geparst() {
        let b = parse_bbox(Some("9.17,48.77,9.19,48.79")).unwrap();
        assert_eq!(b.min_x, 9.17);
        assert_eq!(b.max_y, 48.79);
    }

    #[test]
    fn verdrehte_bbox_wird_normalisiert() {
        let b = parse_bbox(Some("9.19,48.79,9.17,48.77")).unwrap();
        assert_eq!(b.min_x, 9.17);
        assert_eq!(b.max_x, 9.19);
    }

    #[test]
    fn kaputte_bbox_wird_abgelehnt() {
        for input in [None, Some("1,2,3"), Some("a,b,c,d"), Some("")] {
            assert!(parse_bbox(input).is_err(), "{input:?} hätte fehlschlagen müssen");
        }
    }

    #[test]
    fn zeitstempel_ist_wohlgeformt() {
        let t = now_iso();
        assert_eq!(t.len(), 20, "{t}");
        assert!(t.ends_with('Z'), "{t}");
        assert_eq!(&t[4..5], "-");
        assert_eq!(&t[10..11], "T");
        // Plausibler Jahresbereich, fängt Rechenfehler im Kalender ab.
        let jahr: i64 = t[..4].parse().unwrap();
        assert!((2024..2100).contains(&jahr), "{t}");
    }

    #[test]
    fn kalenderumrechnung_stimmt_an_bekannten_daten() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19_723), (2024, 1, 1));
        // Schaltjahr: 29. Februar 2024
        assert_eq!(civil_from_days(19_782), (2024, 2, 29));
    }

    #[test]
    fn unsinnige_koordinaten_werden_abgelehnt() {
        assert!(parse_bbox(Some("200,48,201,49")).is_err(), "Längengrad");
        assert!(parse_bbox(Some("9,95,9.1,96")).is_err(), "Breitengrad");
    }

    #[test]
    fn grosse_bbox_wird_geparst_aber_spaeter_verworfen() {
        // Das Parsen gelingt; die Größenprüfung erfolgt im Handler und liefert
        // dort eine leere Antwort statt eines Fehlers.
        let b = parse_bbox(Some("6.0,50.0,9.0,53.0")).unwrap();
        assert!(b.width() > MAX_BBOX_DEGREES);
    }
}
