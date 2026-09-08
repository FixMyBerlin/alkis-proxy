//! Der eigentliche Datenendpunkt: `/collections/flurstuecke/items`.

use std::sync::atomic::AtomicU64;

use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::api::error::ApiError;
use crate::api::metrics::Metrics;
use crate::api::{base_url, SharedState, TypedJson, GEOJSON};
use crate::config::{states, StateConfig, StateKey};
use crate::crs::reproject::{bbox_to_utm, Bbox};
use crate::crs::tiles;
use crate::routing;

/// Ausschnitt für das Schema-Sample: ein Häuserblock in der Kölner Innenstadt.
/// Bewusst klein und in einem Land mit schnellem, stabilem Dienst; nach dem
/// ersten Abruf liegt er ohnehin im Cache.
const SCHEMA_SAMPLE_BBOX: (f64, f64, f64, f64) = (6.9575, 50.9395, 6.9585, 50.9405);
/// Wie viele Features das Sample umfasst — mehr braucht kein Client, um die
/// Felder zu erkennen.
const SCHEMA_SAMPLE_LIMIT: usize = 2;

/// Grober Vorfilter gegen sinnlose Ausschnitte (Weltkarte, fehlgeschlagene
/// Umprojektion), bevor überhaupt Länder bestimmt werden.
///
/// Die maßgebliche Grenze ist das nicht: Wie viel Fläche eine Anfrage wirklich
/// bedienen kann, entscheidet die Kachelzahl im nativen CRS des jeweiligen
/// Landes — siehe `passt_in_eine_anfrage`. Diese Zahl hier ist nur eine
/// Abkürzung für Fälle, die ohnehin um Größenordnungen daneben liegen.
const ABSURD_BBOX_DEGREES: f64 = 10.0;

#[derive(Debug, Deserialize)]
pub struct ItemsQuery {
    /// `min_lon,min_lat,max_lon,max_lat` in WGS84.
    bbox: Option<String>,
    limit: Option<usize>,
    /// Startversatz in der Treffermenge; Grundlage der Seitennavigation.
    offset: Option<usize>,
    /// Optionale Einschränkung auf ein Bundesland (`NW`). Ohne Angabe
    /// bestimmt der Dienst die zuständigen Länder selbst.
    state: Option<String>,
}

/// Die Anfrage, soweit sie zum Bau der Links gebraucht wird.
///
/// Alle Werte sind normalisiert und stammen aus geprüfter Eingabe — der
/// Rohtext der Anfrage landet nie in einer URL.
struct Page {
    base: String,
    /// Der angefragte Ausschnitt. Beim Schema-Sample keiner, damit die Links
    /// die tatsächlich gestellte Anfrage wiedergeben und nicht den intern
    /// eingesetzten Ersatzausschnitt.
    bbox: Option<Bbox>,
    state: Option<StateKey>,
    limit: usize,
    offset: usize,
}

impl Page {
    /// Diese Ressource mit gegebenem Startversatz.
    fn url(&self, offset: usize) -> String {
        let mut url = format!(
            "{}/collections/flurstuecke/items?limit={}",
            self.base, self.limit
        );
        if let Some(b) = self.bbox {
            url.push_str(&format!(
                "&bbox={},{},{},{}",
                b.min_x, b.min_y, b.max_x, b.max_y
            ));
        }
        if let Some(key) = self.state {
            url.push_str(&format!("&state={}", key.code()));
        }
        if offset > 0 {
            url.push_str(&format!("&offset={offset}"));
        }
        url
    }

    /// `total` ist die gesamte gefundene Treffermenge, `returned` die Zahl der
    /// Objekte in dieser Antwort.
    fn links(&self, total: usize, returned: usize) -> Value {
        let mut links = vec![
            json!({ "rel": "self", "type": GEOJSON, "title": "Diese Antwort", "href": self.url(self.offset) }),
            json!({ "rel": "root", "type": "application/json", "title": "Startseite", "href": self.base }),
            json!({
                "rel": "collection",
                "type": "application/json",
                "title": "Beschreibung der Sammlung",
                "href": format!("{}/collections/flurstuecke", self.base)
            }),
        ];
        // Ohne diesen Link endet der Layer in QGIS nach der ersten Seite: der
        // Provider sucht `next` mit dem Medientyp application/geo+json und
        // hört auf, wenn er ihn nicht findet.
        if self.offset + returned < total {
            links.push(json!({
                "rel": "next",
                "type": GEOJSON,
                "title": "Nächste Seite",
                "href": self.url(self.offset + returned)
            }));
        }
        if self.offset > 0 {
            links.push(json!({
                "rel": "prev",
                "type": GEOJSON,
                "title": "Vorige Seite",
                "href": self.url(self.offset.saturating_sub(self.limit))
            }));
        }
        Value::Array(links)
    }
}

pub async fn items(
    State(app): State<SharedState>,
    headers: HeaderMap,
    Query(q): Query<ItemsQuery>,
) -> Result<TypedJson, ApiError> {
    Metrics::add(&app.metrics.requests, 1);
    let base = base_url(&app, &headers);

    // Die rohe Anfrage, bevor irgendetwas geprüft wird. Nur hier ist zu sehen,
    // ob ein Client überhaupt einen Ausschnitt mitschickt: QGIS hängt eine
    // bbox nur an, wenn in der Verbindung "Nur Objekte im aktuellen
    // Ansichtsbereich abfragen" gesetzt ist — sonst fragt es genau einmal ohne
    // bbox und rendert danach aus seinem eigenen Zwischenspeicher weiter.
    tracing::debug!(
        bbox = q.bbox.as_deref().unwrap_or("—"),
        limit = q.limit.map_or_else(|| "—".to_string(), |v| v.to_string()),
        offset = q.offset.map_or_else(|| "—".to_string(), |v| v.to_string()),
        land = q.state.as_deref().unwrap_or("—"),
        client = header(&headers, "user-agent"),
        "items angefragt"
    );

    // Früh geprüft, damit ein unbekanntes Kürzel auch dann gemeldet wird, wenn
    // die Anfrage aus anderen Gründen leer ausgeht — und damit nur ein
    // kanonisches Kürzel in die Links gerät.
    let state_key = match q.state.as_deref() {
        Some(code) => Some(
            StateKey::from_code(code)
                .ok_or_else(|| ApiError::BadRequest(format!("Unbekanntes Bundesland: {code}")))?,
        ),
        None => None,
    };

    // Ohne Ausschnitt kann bundesweit nichts geliefert werden — es sind rund
    // 60 Millionen Flurstücke. Ein Fehler wäre hier aber falsch: Clients wie
    // QGIS fragen beim Verbinden zunächst ohne bbox an, um das Feldschema zu
    // ermitteln. Bekommen sie dabei nichts, legen sie einen Layer ganz ohne
    // Attributspalten an. Deshalb wird stattdessen ein kleines Sample aus einem
    // festen Ausschnitt geliefert, erkennbar an der beigefügten Warnung.
    let (bbox, is_sample) = match q.bbox.as_deref().filter(|b| !b.trim().is_empty()) {
        Some(raw) => (parse_bbox(Some(raw))?, false),
        None => {
            let (w, s, e, n) = SCHEMA_SAMPLE_BBOX;
            (Bbox::new(w, s, e, n), true)
        }
    };
    let limit = if is_sample {
        SCHEMA_SAMPLE_LIMIT
    } else {
        app.settings.clamp_limit(q.limit)
    };
    let page = Page {
        base,
        bbox: (!is_sample).then_some(bbox),
        state: state_key,
        limit,
        offset: if is_sample { 0 } else { q.offset.unwrap_or(0) },
    };

    // Damit `numberMatched` die Treffermenge beschreibt und nicht bloß die
    // gelieferte Seite, wird unabhängig vom `limit` der Anfrage bis zur
    // Obergrenze des Dienstes eingesammelt und erst danach zugeschnitten. Der
    // erste Abruf eines Ausschnitts kostet dadurch mehr; die Folgeseiten
    // bedient der Kachel-Cache.
    let collect_limit = if is_sample {
        SCHEMA_SAMPLE_LIMIT
    } else {
        app.settings.max_limit
    };

    // Offensichtlich sinnlose Ausschnitte sofort ablehnen, bevor Länder
    // bestimmt werden. Der genaue Grenzfall folgt weiter unten.
    if bbox.width() > ABSURD_BBOX_DEGREES || bbox.height() > ABSURD_BBOX_DEGREES {
        return Err(zu_gross(bbox, None));
    }

    let targets = match state_key {
        Some(key) => {
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
        tracing::info!(
            bbox = %bbox_text(bbox),
            "items leer: Ausschnitt liegt außerhalb der Bundesländer mit Dienst"
        );
        // Kein Fehler: Die BBOX liegt schlicht außerhalb Deutschlands. Hier ist
        // die Treffermenge bekannt, nämlich null.
        return Ok(TypedJson(GEOJSON, empty_collection(&page, Some(0), None)));
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

    // Die eigentliche Größenprüfung: Sie rechnet mit derselben Kachelzerlegung,
    // die der Abruf gleich benutzt, und schlägt deshalb genau dann an, wenn die
    // Anfrage wirklich zu groß wäre. Vorher geprüft, damit gar nichts geladen
    // wird — und damit der Client einen Fehler sieht statt einer leeren
    // Antwort, die seinen Kartencache vergiften würde.
    if let Some(zuviel) = zu_viele_kacheln(&available, bbox) {
        return Err(zu_gross(bbox, Some(zuviel)));
    }

    let begonnen = std::time::Instant::now();
    let outcome = app
        .cache
        .features_for_bbox(&app.client, &app.breaker, &available, bbox, collect_limit)
        .await;

    // Eine Zeile je Anfrage. Ohne sie ist im Betrieb nicht zu unterscheiden,
    // ob eine langsame Antwort am Cache oder an den Landesdiensten liegt —
    // `abrufe` ist die Zahl der tatsächlich gestellten WFS-Anfragen, und die
    // dominiert die Dauer.
    let kacheln = outcome.cache_hits + outcome.cache_misses;
    tracing::info!(
        cache = if app.cache.is_enabled() { "aktiv" } else { "aus" },
        treffer = outcome.cache_hits,
        fehlschlaege = outcome.cache_misses,
        quote = %trefferquote(outcome.cache_hits, kacheln),
        kacheln,
        abrufe = outcome.cache_misses,
        laender = available.len(),
        features = outcome.features.len(),
        ms = begonnen.elapsed().as_millis(),
        // Der Ausschnitt gehört mit ins Log: nur daran ist zu erkennen, ob ein
        // Client überhaupt kartenausschnittsweise anfragt oder immer dieselbe
        // Anfrage stellt.
        bbox = %bbox_text(bbox),
        sample = is_sample,
        "items beantwortet"
    );
    // Warnungen stehen zwar in der Antwort, aber kaum ein Client zeigt sie an —
    // QGIS verwirft sie stillschweigend. Deshalb gehören sie zusätzlich ins Log.
    for w in &outcome.warnings {
        tracing::warn!(warnung = %w, "Teilergebnis");
    }

    Metrics::add(&app.metrics.cache_hits, outcome.cache_hits as u64);
    Metrics::add(&app.metrics.cache_misses, outcome.cache_misses as u64);
    // `features_served` zählt weiter unten, was die Antwort wirklich enthält —
    // eingesammelt wird mehr, als eine Seite ausliefert.
    if !outcome.warnings.is_empty() {
        Metrics::add(&app.metrics.upstream_errors, outcome.warnings.len() as u64);
    }
    warnings.extend(outcome.warnings);
    if is_sample {
        warnings.push(
            "Ohne bbox wird nur ein Beispielausschnitt geliefert, damit Clients das \
             Feldschema erkennen können. Für echte Abfragen bitte bbox angeben \
             (maximal etwa 8 km Kantenlänge)."
                .into(),
        );
    }

    let total = outcome.features.len();
    // `numberMatched` meint nach OGC API Features alle passenden Objekte, nicht
    // die der Seite. Bekannt ist die Zahl nur, wenn zwei Dinge gelten: die
    // Anfrage galt einem echten Ausschnitt (das Sample zählt nicht, sonst
    // erbte QGIS dessen zwei Objekte als Größe des ganzen Layers), und die
    // Sammlung lief nicht in die Obergrenze — dann könnte mehr existieren.
    // Ist sie unbekannt, bleibt das Feld weg; der Standard erlaubt das
    // ausdrücklich und es ist ehrlicher als eine erfundene Zahl.
    let matched = (!is_sample && total < collect_limit).then_some(total);

    let features: Vec<Value> = outcome
        .features
        .into_iter()
        .skip(page.offset)
        .take(page.limit)
        .collect();
    let returned = features.len();
    Metrics::add(&app.metrics.features_served, returned as u64);

    let mut body = json!({
        "type": "FeatureCollection",
        "features": features,
        "numberReturned": returned,
        "timeStamp": now_iso(),
        "links": page.links(total, returned),
    });
    if let Some(matched) = matched {
        body["numberMatched"] = json!(matched);
    }
    if !warnings.is_empty() {
        // Teilausfälle sichtbar machen, statt die Anfrage scheitern zu lassen:
        // ein Ergebnis aus drei von vier Ländern ist meist brauchbar.
        body["warnings"] = json!(warnings);
    }
    // Der Quellenvermerk nennt die Länder, die in dieser Antwort tatsächlich
    // vorkommen — abgelesen an den Features, nicht an den befragten Diensten.
    // Der Unterschied ist keiner auf dem Papier: Die Hüllboxen des Routings
    // überlappen, ein Kölner Ausschnitt befragt deshalb auch Rheinland-Pfalz.
    // Würde man `available` nehmen, stünde unter einer reinen NRW-Karte ein
    // Land, von dem kein einziges Flurstück stammt.
    if let Some(line) = states::attribution_line(&beteiligte_laender(&body), current_year()) {
        body["attribution"] = json!(line);
    }
    Ok(TypedJson(GEOJSON, body))
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

/// Die höchste Kachelzahl, die eines der Länder für diesen Ausschnitt bräuchte
/// — oder `None`, wenn alle unter der Grenze bleiben.
///
/// Geprüft wird je Land, weil jedes sein eigenes natives CRS hat: derselbe
/// Ausschnitt ergibt in Zone 32 und Zone 33 verschieden viele Kacheln.
fn zu_viele_kacheln(states: &[&'static StateConfig], bbox: Bbox) -> Option<usize> {
    states
        .iter()
        .filter_map(|s| s.endpoint)
        .map(|e| tiles::tile_count_for_bbox(bbox_to_utm(bbox, e.native_crs)))
        .filter(|n| *n > tiles::MAX_TILES_PER_REQUEST)
        .max()
}

/// Fehler für einen zu weiten Ausschnitt, mit Angabe der zulässigen Kantenlänge.
///
/// Die Grenze wird in Kilometern genannt statt in Grad: Ein Grad ist je nach
/// Breitengrad und Richtung verschieden lang, eine Kantenlänge in Kilometern
/// kann der Nutzer dagegen unmittelbar mit dem Maßstabsbalken vergleichen.
fn zu_gross(bbox: Bbox, kacheln: Option<usize>) -> ApiError {
    let kante_km =
        (tiles::MAX_TILES_PER_REQUEST as f64).sqrt() * tiles::BASE_TILE_SIZE_M as f64 / 1000.0;
    tracing::info!(
        bbox = %bbox_text(bbox),
        breite_grad = format!("{:.3}", bbox.width()),
        hoehe_grad = format!("{:.3}", bbox.height()),
        kacheln = kacheln.unwrap_or(0),
        grenze_kacheln = tiles::MAX_TILES_PER_REQUEST,
        "items abgelehnt: Ausschnitt zu groß"
    );
    ApiError::TooLarge(format!(
        "Der Ausschnitt ist zu groß. Flurstücke werden bis etwa {kante_km:.0} km Kantenlänge \
         geliefert; bitte näher heranzoomen. In QGIS setzt man dafür am besten eine \
         maßstabsabhängige Sichtbarkeit auf dem Layer."
    ))
}

/// Antwort ohne Objekte. `matched` bleibt `None`, wo unbekannt ist, wie viele
/// Flurstücke der Ausschnitt enthielte.
fn empty_collection(page: &Page, matched: Option<usize>, hint: Option<&str>) -> Value {
    let mut v = json!({
        "type": "FeatureCollection",
        "features": [],
        "numberReturned": 0,
        "timeStamp": now_iso(),
        "links": page.links(0, 0),
    });
    if let Some(matched) = matched {
        v["numberMatched"] = json!(matched);
    }
    if let Some(h) = hint {
        v["warnings"] = json!([h]);
    }
    v
}

/// Die Bundesländer, aus denen die Features dieser Antwort stammen.
///
/// Reihenfolge und Wiederholungen spielen keine Rolle: `attribution_line`
/// sortiert nach Länderschlüssel und nennt jedes Land einmal.
fn beteiligte_laender(body: &Value) -> Vec<StateKey> {
    let mut keys: Vec<StateKey> = body["features"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter_map(|f| f["properties"]["bundesland"].as_str())
        .filter_map(StateKey::from_code)
        .collect();
    keys.sort();
    keys.dedup();
    keys
}

/// Die BBOX so, wie ein Client sie schicken würde — damit eine Logzeile sich
/// unverändert in eine `curl`-Anfrage kopieren lässt.
fn bbox_text(b: Bbox) -> String {
    format!("{},{},{},{}", b.min_x, b.min_y, b.max_x, b.max_y)
}

/// Ein Anfrage-Header als Text, oder `—` wenn er fehlt oder nicht lesbar ist.
fn header(headers: &HeaderMap, name: &str) -> String {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("—")
        .to_string()
}

/// Cache-Trefferquote als Prozenttext fürs Log. Ohne Kacheln (leerer
/// Ausschnitt, alle Länder abgeschaltet) gibt es keine Quote — dann `-`,
/// damit nicht `0%` einen Cache-Fehlschlag vortäuscht, den es nie gab.
fn trefferquote(hits: usize, total: usize) -> String {
    if total == 0 {
        return "-".into();
    }
    format!("{:.0}%", hits as f64 * 100.0 / total as f64)
}

/// Das laufende Jahr (UTC).
///
/// Mehrere Länder verlangen im Quellenvermerk das Jahr des Datenbezugs. Für
/// einen Dienst, der live durchreicht, ist das das Jahr der Anfrage.
pub(crate) fn current_year() -> i64 {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    civil_from_days((secs / 86_400) as i64).0
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
    fn bbox_text_ist_als_anfrageparameter_verwendbar() {
        let b = parse_bbox(Some("9.17,48.77,9.19,48.79")).unwrap();
        assert_eq!(bbox_text(b), "9.17,48.77,9.19,48.79");
        // Und wieder einlesbar — die Logzeile taugt damit zum Nachstellen.
        assert_eq!(parse_bbox(Some(&bbox_text(b))).unwrap().max_y, b.max_y);
    }

    #[test]
    fn trefferquote_ohne_kacheln_ist_kein_nullprozent() {
        assert_eq!(trefferquote(0, 0), "-");
        assert_eq!(trefferquote(0, 4), "0%");
        assert_eq!(trefferquote(3, 4), "75%");
        assert_eq!(trefferquote(4, 4), "100%");
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

    fn seite(offset: usize, limit: usize) -> Page {
        Page {
            base: "http://x".into(),
            bbox: Some(Bbox::new(9.0, 48.0, 9.1, 48.1)),
            state: None,
            limit,
            offset,
        }
    }

    /// Sammelt die `href` je `rel` aus einer Linkliste.
    fn rels(links: &Value) -> Vec<(String, String)> {
        links
            .as_array()
            .unwrap()
            .iter()
            .map(|l| {
                (
                    l["rel"].as_str().unwrap().to_string(),
                    l["href"].as_str().unwrap().to_string(),
                )
            })
            .collect()
    }

    #[test]
    fn next_link_erscheint_nur_wenn_noch_etwas_folgt() {
        let p = seite(0, 100);
        let mit = rels(&p.links(8723, 100));
        let next = mit.iter().find(|(rel, _)| rel == "next").expect("next fehlt");
        assert!(next.1.contains("offset=100"), "{}", next.1);
        assert!(next.1.contains("bbox=9,48,9.1,48.1"), "{}", next.1);

        // Letzte Seite: alles ausgeliefert, also kein Weiter.
        let ohne = rels(&p.links(100, 100));
        assert!(!ohne.iter().any(|(rel, _)| rel == "next"));
    }

    #[test]
    fn prev_link_erscheint_erst_ab_der_zweiten_seite() {
        assert!(!rels(&seite(0, 100).links(500, 100))
            .iter()
            .any(|(rel, _)| rel == "prev"));
        let zweite = rels(&seite(100, 100).links(500, 100));
        let prev = zweite.iter().find(|(rel, _)| rel == "prev").unwrap();
        // Zurück auf offset 0 — und das wird nicht als Parameter geschrieben.
        assert!(!prev.1.contains("offset="), "{}", prev.1);
    }

    #[test]
    fn self_link_gibt_die_gestellte_anfrage_wieder() {
        let p = Page {
            base: "http://x".into(),
            bbox: Some(Bbox::new(9.0, 48.0, 9.1, 48.1)),
            state: StateKey::from_code("NW"),
            limit: 42,
            offset: 84,
        };
        let links = rels(&p.links(500, 42));
        let (_, href) = links.iter().find(|(rel, _)| rel == "self").unwrap();
        assert!(href.contains("limit=42"), "{href}");
        assert!(href.contains("offset=84"), "{href}");
        assert!(href.contains("state=NW"), "{href}");
    }

    #[test]
    fn schema_sample_haengt_keine_bbox_an_die_links() {
        // Ohne bbox angefragt: die Links dürfen den intern eingesetzten
        // Ersatzausschnitt nicht behaupten.
        let p = Page {
            base: "http://x".into(),
            bbox: None,
            state: None,
            limit: SCHEMA_SAMPLE_LIMIT,
            offset: 0,
        };
        let links = rels(&p.links(2, 2));
        assert!(links.iter().all(|(_, href)| !href.contains("bbox=")));
    }

    #[test]
    fn leere_antwort_meldet_die_treffermenge_nur_wenn_bekannt() {
        let p = seite(0, 100);
        // Außerhalb Deutschlands: null Treffer sind eine belastbare Aussage.
        let bekannt = empty_collection(&p, Some(0), None);
        assert_eq!(bekannt["numberMatched"], json!(0));
        assert_eq!(bekannt["numberReturned"], json!(0));

        // Ausschnitt zu groß: die Zahl ist schlicht unbekannt.
        let unbekannt = empty_collection(&p, None, Some("zu groß"));
        assert!(
            unbekannt.get("numberMatched").is_none(),
            "numberMatched darf nicht geraten werden"
        );
        assert_eq!(unbekannt["warnings"][0], json!("zu groß"));
    }

    #[test]
    fn grosse_bbox_wird_geparst_aber_spaeter_verworfen() {
        // Das Parsen gelingt; verworfen wird erst im Handler, anhand der
        // Kachelzahl im nativen CRS.
        let b = parse_bbox(Some("6.0,50.0,9.0,53.0")).unwrap();
        let nw = states::get(StateKey::from_code("NW").unwrap());
        assert_eq!(zu_viele_kacheln(&[nw], b).is_some(), true);
    }

    #[test]
    fn ausschnitt_in_arbeitsgroesse_wird_zugelassen() {
        // Ein Stadtviertel muss durchgehen — sonst wäre der Dienst unbenutzbar.
        let b = parse_bbox(Some("6.95,50.93,6.96,50.94")).unwrap();
        let nw = states::get(StateKey::from_code("NW").unwrap());
        assert_eq!(zu_viele_kacheln(&[nw], b), None);
    }

    /// Der Grund, warum ein zu großer Ausschnitt ein Fehler ist und keine
    /// leere Antwort: Eine erfolgreiche leere Antwort markiert im Kartencache
    /// von QGIS die ganze Region als geladen, und der Layer bleibt danach auch
    /// beim Hineinzoomen leer.
    #[test]
    fn zu_grosser_ausschnitt_ist_ein_fehler_keine_leere_antwort() {
        let b = parse_bbox(Some("6.0,50.0,9.0,53.0")).unwrap();
        let err = zu_gross(b, Some(9_000));
        assert!(matches!(err, ApiError::TooLarge(_)));
        // Die Meldung muss sagen, was zu tun ist.
        assert!(err.to_string().contains("heranzoomen"), "{err}");
    }

    #[test]
    fn quellenvermerk_folgt_den_features_nicht_den_befragten_diensten() {
        // Ein Kölner Ausschnitt befragt wegen überlappender Hüllboxen auch
        // Rheinland-Pfalz. Geliefert hat nur NRW — und nur NRW gehört in den
        // Quellenvermerk.
        let body = json!({
            "features": [
                { "properties": { "bundesland": "NW" } },
                { "properties": { "bundesland": "NW" } }
            ]
        });
        assert_eq!(beteiligte_laender(&body), vec![StateKey::Nw]);
    }

    #[test]
    fn ohne_features_kein_quellenvermerk() {
        assert!(beteiligte_laender(&json!({ "features": [] })).is_empty());
        // Und eine Antwort ohne das Feld darf nicht in Panik geraten.
        assert!(beteiligte_laender(&json!({})).is_empty());
    }

    #[test]
    fn absurde_bbox_wird_vor_der_laenderbestimmung_abgelehnt() {
        // Aus einer fehlgeschlagenen Umprojektion kommen solche Ausschnitte.
        let b = parse_bbox(Some("-53.4,55.2,-11.8,63.1")).unwrap();
        assert!(b.width() > ABSURD_BBOX_DEGREES);
    }
}
