//! Kachelbasierter Read-Through-Cache.
//!
//! Anfragen kommen mit beliebigen Bounding-Boxen, die sich praktisch nie
//! wiederholen. Der Cache arbeitet deshalb nicht auf Antworten, sondern auf
//! Kacheln eines festen Kilometergitters im nativen CRS: Benachbarte
//! Kartenausschnitte teilen sich Kacheln, und die Trefferquote steigt.
//!
//! Zwei Feinheiten, ohne die das Ergebnis still falsch wäre:
//!
//! * **Abgeschnittene Antworten.** Liefert ein Dienst exakt so viele Features,
//!   wie angefordert, ist die Antwort vermutlich unvollständig. Die Kachel wird
//!   dann geviertelt und erneut abgefragt. Ohne das fehlten in Innenstädten
//!   Flurstücke, ohne dass es jemand bemerkt.
//! * **Doppelte Flurstücke.** Ein Flurstück auf einer Kachelgrenze kommt aus
//!   beiden Kacheln. Dedupliziert wird über das kanonische Kennzeichen.
//!
//! Dass eine Kachel geteilt werden muss, wird selbst im Cache vermerkt. Sonst
//! würde sie bei jeder Anfrage erneut abgerufen, nur um wieder festzustellen,
//! dass die Antwort abgeschnitten ist — in dicht bebauten Gebieten also immer.

use std::collections::HashSet;
use std::time::Duration;

use serde_json::Value;

use crate::cache::RedisStore;
use crate::config::StateConfig;
use crate::crs::reproject::{bbox_to_utm, Bbox};
use crate::crs::tiles::{self, Tile};
use crate::upstream::{CircuitBreaker, UpstreamClient};

/// Wie lange eine Kachel mit Inhalt vorgehalten wird. Flurstücksgeometrien
/// ändern sich selten — Teilungen und Verschmelzungen sind pro Gebiet
/// Einzelfälle im Jahr.
pub const HIT_TTL: Duration = Duration::from_secs(30 * 24 * 3600);

/// Leere Kacheln (Wasser, Ausland, unbebautes Gebiet) werden kürzer gehalten,
/// aber sehr wohl gecacht — sonst wird jede leere Fläche bei jedem
/// Kartenausschnitt neu angefragt.
pub const EMPTY_TTL: Duration = Duration::from_secs(7 * 24 * 3600);

/// Wie viele Features pro Kachel höchstens abgefragt werden.
const PER_TILE_LIMIT: usize = 1000;

/// Markierung für eine Kachel, die geteilt werden muss.
const SPLIT_MARKER: &str = "__split";

#[derive(Debug, Default)]
pub struct FetchOutcome {
    /// Die gefundenen Flurstücke als GeoJSON-Features.
    pub features: Vec<Value>,
    /// Hinweise auf Teilausfälle — etwa ein Land, dessen Dienst nicht antwortet.
    /// Die Anfrage schlägt deswegen nicht fehl; ein Teilergebnis ist brauchbarer
    /// als ein Fehler.
    pub warnings: Vec<String>,
    pub cache_hits: usize,
    pub cache_misses: usize,
}

pub struct TileCache {
    store: Option<RedisStore>,
}

impl TileCache {
    pub fn new(store: Option<RedisStore>) -> Self {
        TileCache { store }
    }

    pub fn is_enabled(&self) -> bool {
        self.store.is_some()
    }

    /// Holt alle Flurstücke in der BBOX (WGS84) aus den zuständigen Ländern.
    pub async fn features_for_bbox(
        &self,
        client: &UpstreamClient,
        breaker: &CircuitBreaker,
        states: &[&'static StateConfig],
        bbox: Bbox,
        limit: usize,
    ) -> FetchOutcome {
        let mut out = FetchOutcome::default();
        let mut seen: HashSet<String> = HashSet::new();

        for state in states {
            let Some(endpoint) = state.endpoint else {
                continue;
            };
            let crs = endpoint.native_crs;
            let native = bbox_to_utm(bbox, crs);

            let base_tiles = match tiles::tiles_for_bbox(native) {
                Ok(t) => t,
                Err(e) => {
                    out.warnings.push(e.to_string());
                    continue;
                }
            };

            // Arbeitsvorrat, damit geteilte Kacheln ohne Rekursion nachrücken.
            let mut queue: Vec<Tile> = base_tiles;
            while let Some(tile) = queue.pop() {
                match self.tile_features(client, state, tile).await {
                    Ok(TileLoad { features, truncated, from_cache }) => {
                        if from_cache {
                            out.cache_hits += 1;
                        } else {
                            out.cache_misses += 1;
                            breaker.record_success(state.key);
                        }
                        if truncated {
                            if let Some(quads) = tile.subdivide() {
                                tracing::debug!(
                                    state = state.key.code(),
                                    key = %tile.cache_key(state.key, crs),
                                    "Antwort abgeschnitten, Kachel wird geteilt"
                                );
                                queue.extend_from_slice(&quads);
                                continue;
                            }
                            // Kleinste Kachel und immer noch abgeschnitten:
                            // liefern, was da ist, aber sichtbar machen.
                            out.warnings.push(format!(
                                "{}: Ausschnitt zu dicht bebaut, Ergebnis unvollständig",
                                state.label
                            ));
                        }
                        for f in features {
                            let Some(id) = feature_id(&f) else { continue };
                            if !intersects(&f, bbox) {
                                continue;
                            }
                            if seen.insert(id) {
                                out.features.push(f);
                            }
                        }
                    }
                    Err(message) => {
                        breaker.record_failure(state.key);
                        out.warnings.push(message);
                        // Ein ausgefallenes Land beendet die Anfrage nicht — die
                        // übrigen liefern weiter, das Ergebnis wird als
                        // unvollständig gekennzeichnet.
                        break;
                    }
                }

                // Schutz gegen Ausreißer, aber großzügig: Erst nach dem
                // Einsammeln wird sortiert und gekürzt, damit das Ergebnis
                // nicht von der Kachelreihenfolge abhängt.
                if out.features.len() >= limit.saturating_mul(4) {
                    break;
                }
            }
        }

        // Stabile Reihenfolge über das kanonische Kennzeichen. Ohne sie liefert
        // dieselbe Anfrage je nach Cache-Zustand und Antwortreihenfolge der
        // Dienste unterschiedliche Ausschnitte, sobald das Limit greift.
        out.features.sort_by(|a, b| {
            feature_id(a)
                .unwrap_or_default()
                .cmp(&feature_id(b).unwrap_or_default())
        });
        out.features.truncate(limit);
        out
    }

    /// Eine Kachel — aus dem Cache oder vom Landesdienst.
    async fn tile_features(
        &self,
        client: &UpstreamClient,
        state: &'static StateConfig,
        tile: Tile,
    ) -> Result<TileLoad, String> {
        let endpoint = state.endpoint.ok_or_else(|| {
            format!("{}: kein Endpunkt konfiguriert", state.label)
        })?;
        let key = tile.cache_key(state.key, endpoint.native_crs);

        if let Some(store) = &self.store {
            if let Some(raw) = store.get(&key).await {
                match serde_json::from_slice::<Value>(&raw) {
                    // Bekannt zu dichte Kachel: sofort teilen, ohne Abruf.
                    Ok(v) if v.get(SPLIT_MARKER).is_some() => {
                        return Ok(TileLoad {
                            features: Vec::new(),
                            truncated: true,
                            from_cache: true,
                        })
                    }
                    Ok(Value::Array(features)) => {
                        return Ok(TileLoad {
                            features,
                            truncated: false,
                            from_cache: true,
                        })
                    }
                    _ => {}
                }
            }
        }

        let result = client
            .fetch(state, tile.bbox(), PER_TILE_LIMIT)
            .await
            .map_err(|e| e.to_string())?;

        let features: Vec<Value> = result.parcels.iter().map(|p| p.to_feature()).collect();

        if let Some(store) = &self.store {
            if result.truncated {
                // Nicht die unvollständige Antwort speichern, sondern die
                // Erkenntnis, dass hier geteilt werden muss.
                let marker = serde_json::json!({ SPLIT_MARKER: true });
                if let Ok(bytes) = serde_json::to_vec(&marker) {
                    store.set(&key, &bytes, HIT_TTL).await;
                }
            } else if let Ok(bytes) = serde_json::to_vec(&features) {
                let ttl = if features.is_empty() { EMPTY_TTL } else { HIT_TTL };
                store.set(&key, &bytes, ttl).await;
            }
        }

        Ok(TileLoad {
            features,
            truncated: result.truncated,
            from_cache: false,
        })
    }
}

struct TileLoad {
    features: Vec<Value>,
    truncated: bool,
    from_cache: bool,
}

/// Kanonisches Kennzeichen eines Features — der Schlüssel für die Deduplizierung.
fn feature_id(f: &Value) -> Option<String> {
    f.get("id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            f.pointer("/properties/parcelId")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
}

/// Ob die Geometrie des Features die angefragte BBOX berührt.
///
/// Kacheln reichen über den angefragten Ausschnitt hinaus; ohne diesen Filter
/// bekämen Clients Flurstücke weit außerhalb ihrer BBOX geliefert. Verglichen
/// werden die Hüllboxen — das ist großzügig, aber nie zu eng.
fn intersects(f: &Value, bbox: Bbox) -> bool {
    let Some(coords) = f.pointer("/geometry/coordinates") else {
        return false;
    };
    let (mut min_x, mut min_y) = (f64::MAX, f64::MAX);
    let (mut max_x, mut max_y) = (f64::MIN, f64::MIN);
    let mut any = false;

    collect_positions(coords, &mut |x, y| {
        any = true;
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x);
        max_y = max_y.max(y);
    });

    any && min_x <= bbox.max_x && max_x >= bbox.min_x && min_y <= bbox.max_y && max_y >= bbox.min_y
}

/// Läuft durch beliebig tief verschachtelte GeoJSON-Koordinaten.
fn collect_positions(v: &Value, f: &mut impl FnMut(f64, f64)) {
    let Some(arr) = v.as_array() else { return };
    // Eine Position ist ein Array, dessen erste beiden Elemente Zahlen sind.
    if let (Some(Value::Number(x)), Some(Value::Number(y))) = (arr.first(), arr.get(1)) {
        if let (Some(x), Some(y)) = (x.as_f64(), y.as_f64()) {
            f(x, y);
            return;
        }
    }
    for item in arr {
        collect_positions(item, f);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn feature(id: &str, ring: Vec<[f64; 2]>) -> Value {
        json!({
            "type": "Feature",
            "id": id,
            "geometry": { "type": "MultiPolygon", "coordinates": [[ring]] },
            "properties": { "parcelId": id }
        })
    }

    #[test]
    fn feature_id_kommt_aus_id_oder_properties() {
        assert_eq!(
            feature_id(&feature("NW:123", vec![[0.0, 0.0]])).as_deref(),
            Some("NW:123")
        );
        let ohne_id = json!({"type":"Feature","properties":{"parcelId":"X"}});
        assert_eq!(feature_id(&ohne_id).as_deref(), Some("X"));
        assert_eq!(feature_id(&json!({"type":"Feature"})), None);
    }

    #[test]
    fn features_ausserhalb_der_bbox_werden_gefiltert() {
        let bbox = Bbox::new(9.0, 48.0, 9.1, 48.1);
        let drin = feature("a", vec![[9.05, 48.05], [9.06, 48.06], [9.05, 48.05]]);
        let draussen = feature("b", vec![[10.0, 49.0], [10.1, 49.1], [10.0, 49.0]]);
        assert!(intersects(&drin, bbox));
        assert!(!intersects(&draussen, bbox));
    }

    #[test]
    fn teilweise_ueberlappende_features_bleiben_erhalten() {
        // Ein Flurstück, das über den Rand hinausragt, gehört ins Ergebnis.
        let bbox = Bbox::new(9.0, 48.0, 9.1, 48.1);
        let ueberlappend = feature("c", vec![[9.09, 48.09], [9.2, 48.2], [9.09, 48.09]]);
        assert!(intersects(&ueberlappend, bbox));
    }

    #[test]
    fn geometrie_ohne_koordinaten_wird_verworfen() {
        let leer = json!({"type":"Feature","id":"x","geometry":{"type":"MultiPolygon","coordinates":[]}});
        assert!(!intersects(&leer, Bbox::new(0.0, 0.0, 1.0, 1.0)));
    }

    #[test]
    fn positionen_werden_aus_jeder_verschachtelung_gelesen() {
        let mut punkte = Vec::new();
        collect_positions(
            &json!([[[[1.0, 2.0], [3.0, 4.0]]], [[[5.0, 6.0]]]]),
            &mut |x, y| punkte.push((x, y)),
        );
        assert_eq!(punkte, vec![(1.0, 2.0), (3.0, 4.0), (5.0, 6.0)]);
    }

    #[test]
    fn cache_ist_optional() {
        let c = TileCache::new(None);
        assert!(!c.is_enabled());
    }

    #[test]
    fn leere_kacheln_leben_kuerzer_als_gefuellte() {
        assert!(EMPTY_TTL < HIT_TTL);
    }
}
