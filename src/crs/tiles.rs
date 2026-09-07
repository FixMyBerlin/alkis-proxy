//! Kachelraster im nativen CRS.
//!
//! Anfragen kommen mit beliebigen Bounding-Boxen herein, die sich praktisch nie
//! exakt wiederholen — als Cache-Schlüssel taugen sie deshalb nicht. Der Dienst
//! rastert sie stattdessen auf ein festes Kilometergitter im nativen CRS des
//! Landes. Benachbarte Kartenausschnitte teilen sich dadurch Kacheln, und der
//! Cache greift.
//!
//! Kacheln können geteilt werden: Liefert eine Quelle exakt so viele Features,
//! wie angefordert wurden, ist die Antwort vermutlich abgeschnitten. Dann wird
//! die Kachel geviertelt und erneut angefragt, bis die Antwort vollständig ist
//! oder die maximale Tiefe erreicht ist.

use crate::config::{Crs, StateKey};
use crate::crs::reproject::Bbox;

/// Kantenlänge der größten Kachel in Metern.
pub const BASE_TILE_SIZE_M: u32 = 1000;

/// Wie oft eine Kachel höchstens geteilt werden darf (1000 m → 125 m).
pub const MAX_SUBDIVISION_DEPTH: u8 = 3;

/// Obergrenze für die Kachelanzahl einer einzelnen Anfrage.
///
/// Schützt davor, dass eine sehr große BBOX hunderte Anfragen an einen
/// Landesdienst auslöst.
pub const MAX_TILES_PER_REQUEST: usize = 256;

/// Eine Rasterkachel. Koordinaten sind Vielfache von `size_m` im nativen CRS.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Tile {
    pub x: i32,
    pub y: i32,
    pub size_m: u32,
}

impl Tile {
    /// Kachel, die den Punkt enthält.
    pub fn containing(x: f64, y: f64, size_m: u32) -> Self {
        Tile {
            x: (x / size_m as f64).floor() as i32,
            y: (y / size_m as f64).floor() as i32,
            size_m,
        }
    }

    /// Ausdehnung der Kachel im nativen CRS.
    pub fn bbox(&self) -> Bbox {
        let s = self.size_m as f64;
        Bbox {
            min_x: self.x as f64 * s,
            min_y: self.y as f64 * s,
            max_x: (self.x + 1) as f64 * s,
            max_y: (self.y + 1) as f64 * s,
        }
    }

    /// Teilt die Kachel in vier gleich große Viertel.
    ///
    /// Gibt `None` zurück, wenn die kleinste zulässige Größe erreicht ist.
    pub fn subdivide(&self) -> Option<[Tile; 4]> {
        let half = self.size_m / 2;
        if half < BASE_TILE_SIZE_M >> MAX_SUBDIVISION_DEPTH {
            return None;
        }
        let (x, y) = (self.x * 2, self.y * 2);
        Some([
            Tile { x, y, size_m: half },
            Tile { x: x + 1, y, size_m: half },
            Tile { x, y: y + 1, size_m: half },
            Tile { x: x + 1, y: y + 1, size_m: half },
        ])
    }

    /// Wie oft diese Kachel gegenüber der Basisgröße bereits geteilt wurde.
    pub fn depth(&self) -> u8 {
        let mut size = BASE_TILE_SIZE_M;
        let mut depth = 0;
        while size > self.size_m && depth < MAX_SUBDIVISION_DEPTH {
            size /= 2;
            depth += 1;
        }
        depth
    }

    /// Cache-Schlüssel.
    ///
    /// Enthält **das Bundesland**, weil sich die Hüllboxen benachbarter Länder
    /// überlappen: Erfurt etwa liegt in den Boxen von Thüringen und
    /// Sachsen-Anhalt. Ohne das Kürzel würde die leere Antwort des einen Landes
    /// als Treffer für das andere gelesen — und dessen Flurstücke blieben
    /// unsichtbar.
    ///
    /// Enthält außerdem das CRS, weil dieselben Rasterkoordinaten in Zone 32
    /// und 33 verschiedene Orte bezeichnen.
    pub fn cache_key(&self, state: StateKey, crs: Crs) -> String {
        format!(
            "alkis:v1:{}:{}:{}:{}:{}",
            state.code(),
            crs.code(),
            self.size_m,
            self.x,
            self.y
        )
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TileError {
    #[error("Die Anfrage überdeckt {got} Kacheln, erlaubt sind {MAX_TILES_PER_REQUEST}. Bitte den Ausschnitt verkleinern.")]
    TooManyTiles { got: usize },
}

/// Alle Basiskacheln, die die BBOX berühren.
pub fn tiles_for_bbox(bbox: Bbox) -> Result<Vec<Tile>, TileError> {
    tiles_for_bbox_sized(bbox, BASE_TILE_SIZE_M)
}

/// Wie viele Basiskacheln die BBOX berührt — ohne sie aufzubauen.
///
/// Wird vor dem Abruf gebraucht: Der Handler muss eine zu große Anfrage
/// ablehnen können, bevor irgendetwas geladen wird.
pub fn tile_count_for_bbox(bbox: Bbox) -> usize {
    let (_, _, _, _, count) = tile_span(bbox, BASE_TILE_SIZE_M);
    count
}

/// Kachelbereich der BBOX als `(x0, y0, x1, y1, anzahl)`.
///
/// Die Zählung rechnet in `i64` und sättigt: Eine BBOX aus einer
/// fehlgeschlagenen Umprojektion kann Koordinaten weit jenseits des
/// Gültigkeitsbereichs enthalten, und ein Überlauf würde daraus eine
/// harmlos kleine Kachelzahl machen — die Anfrage liefe dann doch los.
fn tile_span(bbox: Bbox, size_m: u32) -> (i64, i64, i64, i64, usize) {
    let s = size_m as f64;
    let clamp = |v: f64| v.clamp(i32::MIN as f64, i32::MAX as f64) as i64;
    let x0 = clamp((bbox.min_x / s).floor());
    let y0 = clamp((bbox.min_y / s).floor());
    // Eine exakt auf der Kachelgrenze endende BBOX soll die angrenzende Kachel
    // nicht mitnehmen, deshalb ceil() - 1 statt floor().
    let x1 = clamp((bbox.max_x / s).ceil() - 1.0).max(x0);
    let y1 = clamp((bbox.max_y / s).ceil() - 1.0).max(y0);

    let breite = (x1 - x0).saturating_add(1);
    let hoehe = (y1 - y0).saturating_add(1);
    // Nach dem Sättigen bleibt der Wert im i64-Bereich und ist nie negativ;
    // er passt damit in usize.
    let count = breite.saturating_mul(hoehe).max(0) as usize;
    (x0, y0, x1, y1, count)
}

fn tiles_for_bbox_sized(bbox: Bbox, size_m: u32) -> Result<Vec<Tile>, TileError> {
    let (x0, y0, x1, y1, count) = tile_span(bbox, size_m);
    if count > MAX_TILES_PER_REQUEST {
        return Err(TileError::TooManyTiles { got: count });
    }

    let mut tiles = Vec::with_capacity(count);
    for y in y0..=y1 {
        for x in x0..=x1 {
            tiles.push(Tile { x: x as i32, y: y as i32, size_m });
        }
    }
    Ok(tiles)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kachel_enthaelt_ihren_punkt() {
        let t = Tile::containing(356_535.0, 5_645_138.0, BASE_TILE_SIZE_M);
        assert_eq!(t.x, 356);
        assert_eq!(t.y, 5645);
        let b = t.bbox();
        assert!(b.contains(356_535.0, 5_645_138.0));
        assert_eq!(b.min_x, 356_000.0);
        assert_eq!(b.max_y, 5_646_000.0);
    }

    #[test]
    fn kleine_bbox_ergibt_wenige_kacheln() {
        // 500 m Kantenlänge, mitten in einer Kachel.
        let b = Bbox::new(356_200.0, 5_645_200.0, 356_700.0, 5_645_700.0);
        let tiles = tiles_for_bbox(b).unwrap();
        assert_eq!(tiles.len(), 1);
        assert_eq!(tiles[0], Tile { x: 356, y: 5645, size_m: 1000 });
    }

    #[test]
    fn bbox_ueber_kachelgrenze_ergibt_vier_kacheln() {
        let b = Bbox::new(355_900.0, 5_644_900.0, 356_100.0, 5_645_100.0);
        let tiles = tiles_for_bbox(b).unwrap();
        assert_eq!(tiles.len(), 4);
    }

    #[test]
    fn bbox_genau_auf_der_grenze_nimmt_keine_zusatzkachel() {
        let b = Bbox::new(356_000.0, 5_645_000.0, 357_000.0, 5_646_000.0);
        let tiles = tiles_for_bbox(b).unwrap();
        assert_eq!(tiles.len(), 1, "erhielt {tiles:?}");
    }

    #[test]
    fn kacheln_ueberdecken_die_bbox_vollstaendig() {
        let b = Bbox::new(356_432.0, 5_645_111.0, 359_876.0, 5_648_222.0);
        let tiles = tiles_for_bbox(b).unwrap();
        // Jede Ecke der BBOX muss in einer der Kacheln liegen.
        for (x, y) in [
            (b.min_x, b.min_y),
            (b.max_x, b.min_y),
            (b.min_x, b.max_y),
            (b.max_x, b.max_y),
        ] {
            assert!(
                tiles.iter().any(|t| t.bbox().contains(x, y)),
                "({x}, {y}) nicht abgedeckt"
            );
        }
    }

    #[test]
    fn zu_grosse_bbox_wird_abgelehnt() {
        let b = Bbox::new(0.0, 0.0, 100_000.0, 100_000.0); // 10 000 Kacheln
        assert!(matches!(
            tiles_for_bbox(b),
            Err(TileError::TooManyTiles { .. })
        ));
    }

    #[test]
    fn teilung_viertelt_die_flaeche_lueckenlos() {
        let t = Tile { x: 356, y: 5645, size_m: 1000 };
        let quads = t.subdivide().expect("teilbar");
        assert_eq!(quads.len(), 4);
        let parent = t.bbox();
        for q in &quads {
            assert_eq!(q.size_m, 500);
            let qb = q.bbox();
            assert!(qb.min_x >= parent.min_x && qb.max_x <= parent.max_x);
            assert!(qb.min_y >= parent.min_y && qb.max_y <= parent.max_y);
        }
        // Summe der Teilflächen entspricht der Elternfläche.
        let sum: f64 = quads.iter().map(|q| q.bbox().width() * q.bbox().height()).sum();
        assert!((sum - parent.width() * parent.height()).abs() < 1.0);
    }

    #[test]
    fn teilung_endet_bei_der_kleinsten_groesse() {
        let mut t = Tile { x: 0, y: 0, size_m: BASE_TILE_SIZE_M };
        let mut steps = 0;
        while let Some(q) = t.subdivide() {
            t = q[0];
            steps += 1;
            assert!(steps <= MAX_SUBDIVISION_DEPTH, "Teilung endet nicht");
        }
        assert_eq!(steps, MAX_SUBDIVISION_DEPTH);
        assert_eq!(t.size_m, 125);
        assert_eq!(t.depth(), MAX_SUBDIVISION_DEPTH);
    }

    #[test]
    fn cache_key_trennt_die_utm_zonen() {
        let t = Tile { x: 356, y: 5645, size_m: 1000 };
        let k32 = t.cache_key(StateKey::Nw, Crs::Epsg25832);
        let k33 = t.cache_key(StateKey::Nw, Crs::Epsg25833);
        assert_ne!(k32, k33);
        assert_eq!(k32, "alkis:v1:NW:25832:1000:356:5645");
    }

    #[test]
    fn cache_key_trennt_die_bundeslaender() {
        // Erfurt liegt in den Hüllboxen von Thüringen und Sachsen-Anhalt, beide
        // in Zone 32. Teilten sie sich einen Schlüssel, würde die leere Antwort
        // des einen die Flurstücke des anderen verdecken.
        let t = Tile { x: 642, y: 5649, size_m: 1000 };
        assert_ne!(
            t.cache_key(StateKey::Th, Crs::Epsg25832),
            t.cache_key(StateKey::St, Crs::Epsg25832)
        );
    }

    #[test]
    fn negative_koordinaten_runden_korrekt() {
        // Sollte im Bundesgebiet nicht vorkommen, darf aber nicht falsch runden.
        let t = Tile::containing(-1.0, -1.0, 1000);
        assert_eq!((t.x, t.y), (-1, -1));
        assert!(t.bbox().contains(-1.0, -1.0));
    }
}
