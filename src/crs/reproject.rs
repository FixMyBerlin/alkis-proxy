//! Umrechnung zwischen ETRS89/UTM (EPSG:25832, EPSG:25833) und WGS84 (EPSG:4326).
//!
//! Warum eigene Implementierung statt einer Bibliothek: Gebraucht wird genau ein
//! Fall — transversale Mercator-Projektion auf dem GRS80-Ellipsoid, zwei Zonen.
//! Die klassischen Reihenentwicklungen liefern dafür Millimetergenauigkeit
//! innerhalb der Zonenbreite, und der Dienst bleibt frei von nativen
//! Abhängigkeiten (relevant für ein statisch gelinktes Container-Image).
//!
//! ETRS89 und WGS84 werden hier gleichgesetzt. Sie driften mit der eurasischen
//! Platte auseinander (aktuell gut ein halber Meter), was für die Darstellung
//! und Auswahl von Flurstücken ohne Belang ist. Wer katastergenaue
//! Koordinaten braucht, fragt die Quelle direkt im nativen CRS ab.
//!
//! Die Implementierung ist gegen `cs2cs` (PROJ) verifiziert; siehe Tests.

use crate::config::Crs;

// GRS80 — das Ellipsoid von ETRS89.
const A: f64 = 6_378_137.0;
const F: f64 = 1.0 / 298.257_222_101;
/// Maßstabsfaktor am Mittelmeridian (UTM).
const K0: f64 = 0.9996;
/// Rechtswert-Verschiebung (false easting).
const FALSE_EASTING: f64 = 500_000.0;

/// Eine Bounding-Box als `[min_x, min_y, max_x, max_y]`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bbox {
    pub min_x: f64,
    pub min_y: f64,
    pub max_x: f64,
    pub max_y: f64,
}

impl Bbox {
    pub fn new(min_x: f64, min_y: f64, max_x: f64, max_y: f64) -> Self {
        Bbox {
            min_x: min_x.min(max_x),
            min_y: min_y.min(max_y),
            max_x: min_x.max(max_x),
            max_y: min_y.max(max_y),
        }
    }

    pub fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.min_x && x <= self.max_x && y >= self.min_y && y <= self.max_y
    }

    pub fn width(&self) -> f64 {
        self.max_x - self.min_x
    }

    pub fn height(&self) -> f64 {
        self.max_y - self.min_y
    }
}

/// Mittelmeridian der Zone in Grad.
fn central_meridian(crs: Crs) -> f64 {
    match crs {
        Crs::Epsg25832 => 9.0,
        Crs::Epsg25833 => 15.0,
    }
}

fn e_sq() -> f64 {
    2.0 * F - F * F
}

/// Geographisch (Länge, Breite in Grad) → UTM (Rechts-, Hochwert in Metern).
pub fn wgs84_to_utm(lon: f64, lat: f64, crs: Crs) -> (f64, f64) {
    let e2 = e_sq();
    let ep2 = e2 / (1.0 - e2);

    let phi = lat.to_radians();
    let lambda = lon.to_radians();
    let lambda0 = central_meridian(crs).to_radians();

    let (sin_phi, cos_phi) = phi.sin_cos();
    let tan_phi = phi.tan();

    let n = A / (1.0 - e2 * sin_phi * sin_phi).sqrt();
    let t = tan_phi * tan_phi;
    let c = ep2 * cos_phi * cos_phi;
    let a_ = (lambda - lambda0) * cos_phi;

    let m = meridian_arc(phi, e2);

    let a2 = a_ * a_;
    let x = FALSE_EASTING
        + K0 * n
            * (a_ + (1.0 - t + c) * a_ * a2 / 6.0
                + (5.0 - 18.0 * t + t * t + 72.0 * c - 58.0 * ep2) * a_ * a2 * a2 / 120.0);

    let y = K0
        * (m + n
            * tan_phi
            * (a2 / 2.0
                + (5.0 - t + 9.0 * c + 4.0 * c * c) * a2 * a2 / 24.0
                + (61.0 - 58.0 * t + t * t + 600.0 * c - 330.0 * ep2) * a2 * a2 * a2 / 720.0));

    (x, y)
}

/// UTM (Rechts-, Hochwert in Metern) → geographisch (Länge, Breite in Grad).
pub fn utm_to_wgs84(x: f64, y: f64, crs: Crs) -> (f64, f64) {
    let e2 = e_sq();
    let ep2 = e2 / (1.0 - e2);

    let m = y / K0;
    let mu = m / (A * (1.0 - e2 / 4.0 - 3.0 * e2 * e2 / 64.0 - 5.0 * e2 * e2 * e2 / 256.0));

    let e1 = (1.0 - (1.0 - e2).sqrt()) / (1.0 + (1.0 - e2).sqrt());
    let e1_2 = e1 * e1;
    let e1_3 = e1_2 * e1;
    let e1_4 = e1_3 * e1;

    let phi1 = mu
        + (3.0 * e1 / 2.0 - 27.0 * e1_3 / 32.0) * (2.0 * mu).sin()
        + (21.0 * e1_2 / 16.0 - 55.0 * e1_4 / 32.0) * (4.0 * mu).sin()
        + (151.0 * e1_3 / 96.0) * (6.0 * mu).sin()
        + (1097.0 * e1_4 / 512.0) * (8.0 * mu).sin();

    let (sin_phi1, cos_phi1) = phi1.sin_cos();
    let tan_phi1 = phi1.tan();

    let c1 = ep2 * cos_phi1 * cos_phi1;
    let t1 = tan_phi1 * tan_phi1;
    let denom = 1.0 - e2 * sin_phi1 * sin_phi1;
    let n1 = A / denom.sqrt();
    let r1 = A * (1.0 - e2) / (denom * denom.sqrt());
    let d = (x - FALSE_EASTING) / (n1 * K0);

    let d2 = d * d;
    let phi = phi1
        - (n1 * tan_phi1 / r1)
            * (d2 / 2.0
                - (5.0 + 3.0 * t1 + 10.0 * c1 - 4.0 * c1 * c1 - 9.0 * ep2) * d2 * d2 / 24.0
                + (61.0 + 90.0 * t1 + 298.0 * c1 + 45.0 * t1 * t1
                    - 252.0 * ep2
                    - 3.0 * c1 * c1)
                    * d2 * d2 * d2
                    / 720.0);

    let lambda = (d - (1.0 + 2.0 * t1 + c1) * d * d2 / 6.0
        + (5.0 - 2.0 * c1 + 28.0 * t1 - 3.0 * c1 * c1 + 8.0 * ep2 + 24.0 * t1 * t1) * d * d2 * d2
            / 120.0)
        / cos_phi1;

    let lon = central_meridian(crs) + lambda.to_degrees();
    (lon, phi.to_degrees())
}

/// Meridianbogenlänge vom Äquator bis zur Breite `phi`.
fn meridian_arc(phi: f64, e2: f64) -> f64 {
    let e4 = e2 * e2;
    let e6 = e4 * e2;
    A * ((1.0 - e2 / 4.0 - 3.0 * e4 / 64.0 - 5.0 * e6 / 256.0) * phi
        - (3.0 * e2 / 8.0 + 3.0 * e4 / 32.0 + 45.0 * e6 / 1024.0) * (2.0 * phi).sin()
        + (15.0 * e4 / 256.0 + 45.0 * e6 / 1024.0) * (4.0 * phi).sin()
        - (35.0 * e6 / 3072.0) * (6.0 * phi).sin())
}

/// Rechnet eine WGS84-BBOX ins native CRS um.
///
/// Es genügt nicht, nur die beiden Eckpunkte zu transformieren: Meridiane bilden
/// in UTM keine senkrechten Linien ab, sodass die Kanten leicht gekrümmt sind.
/// Deshalb werden alle vier Ecken und die Kantenmitten einbezogen und die
/// umschließende Box gebildet — sonst fehlten an den Rändern Flurstücke.
pub fn bbox_to_utm(bbox: Bbox, crs: Crs) -> Bbox {
    let mid_x = (bbox.min_x + bbox.max_x) / 2.0;
    let mid_y = (bbox.min_y + bbox.max_y) / 2.0;
    let samples = [
        (bbox.min_x, bbox.min_y),
        (bbox.max_x, bbox.min_y),
        (bbox.min_x, bbox.max_y),
        (bbox.max_x, bbox.max_y),
        (mid_x, bbox.min_y),
        (mid_x, bbox.max_y),
        (bbox.min_x, mid_y),
        (bbox.max_x, mid_y),
    ];

    let mut out: Option<Bbox> = None;
    for (lon, lat) in samples {
        let (x, y) = wgs84_to_utm(lon, lat, crs);
        out = Some(match out {
            None => Bbox {
                min_x: x,
                min_y: y,
                max_x: x,
                max_y: y,
            },
            Some(b) => Bbox {
                min_x: b.min_x.min(x),
                min_y: b.min_y.min(y),
                max_x: b.max_x.max(x),
                max_y: b.max_y.max(y),
            },
        });
    }
    out.expect("mindestens ein Stützpunkt")
}

/// Nachkommastellen der ausgegebenen Koordinaten.
///
/// Sieben Stellen entsprechen in Deutschland gut einem Zentimeter — mehr als
/// genug für Flurstücksgrenzen, deren Erfassungsgenauigkeit im Dezimeterbereich
/// liegt. Das Runden hat zwei weitere Vorteile: Die Antworten werden rund ein
/// Drittel kleiner (`9.1792547` statt `9.179254737370037`), und identische
/// Anfragen liefern bitgleiche Ergebnisse, statt sich im Fließkommarauschen der
/// letzten Stellen zu unterscheiden.
const OUTPUT_DECIMALS: f64 = 1e7;

fn round_coord(v: f64) -> f64 {
    (v * OUTPUT_DECIMALS).round() / OUTPUT_DECIMALS
}

/// Rechnet eine Geometrie vom nativen CRS nach WGS84 um (in place).
pub fn geometry_to_wgs84(geometry: &mut crate::model::Geometry, crs: Crs) {
    for polygon in &mut geometry.polygons {
        for ring in polygon.iter_mut() {
            for point in ring.iter_mut() {
                let (lon, lat) = utm_to_wgs84(point[0], point[1], crs);
                *point = [round_coord(lon), round_coord(lat)];
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Referenzwerte aus `cs2cs` (PROJ), auf ganze Meter gerundet.
    /// Erzeugt mit: `echo "<lat> <lon>" | cs2cs EPSG:4326 EPSG:2583x -f "%.0f"`
    const REFERENZ: &[(&str, f64, f64, Crs, f64, f64)] = &[
        ("Köln", 6.958, 50.940, Crs::Epsg25832, 356_535.0, 5_645_138.0),
        ("Wiesbaden", 8.240, 50.080, Crs::Epsg25832, 445_624.0, 5_547_802.0),
        ("Erfurt", 11.030, 50.980, Crs::Epsg25832, 642_499.0, 5_649_562.0),
        ("Dresden", 13.740, 51.050, Crs::Epsg25833, 411_683.0, 5_656_140.0),
        ("Schwerin", 11.420, 53.630, Crs::Epsg25833, 263_302.0, 5_948_314.0),
        ("Potsdam", 13.060, 52.390, Crs::Epsg25833, 367_985.0, 5_806_187.0),
    ];

    #[test]
    fn hinrichtung_stimmt_mit_proj_ueberein() {
        for (name, lon, lat, crs, ex, ey) in REFERENZ {
            let (x, y) = wgs84_to_utm(*lon, *lat, *crs);
            assert!(
                (x - ex).abs() < 1.0,
                "{name}: Rechtswert {x:.1} weicht von PROJ ({ex}) um {:.2} m ab",
                (x - ex).abs()
            );
            assert!(
                (y - ey).abs() < 1.0,
                "{name}: Hochwert {y:.1} weicht von PROJ ({ey}) um {:.2} m ab",
                (y - ey).abs()
            );
        }
    }

    #[test]
    fn rueckrichtung_stimmt_mit_proj_ueberein() {
        for (name, lon, lat, crs, ex, ey) in REFERENZ {
            let (glon, glat) = utm_to_wgs84(*ex, *ey, *crs);
            // 1 m entspricht rund 1.4e-5 ° Länge auf dieser Breite.
            assert!(
                (glon - lon).abs() < 2e-5,
                "{name}: Länge {glon:.7} statt {lon}"
            );
            assert!(
                (glat - lat).abs() < 2e-5,
                "{name}: Breite {glat:.7} statt {lat}"
            );
        }
    }

    #[test]
    fn hin_und_zurueck_bleibt_im_millimeterbereich() {
        // Die Reihenentwicklungen sind endlich, ein Restfehler ist unvermeidlich.
        // Gemessen liegt er zwischen 0,003 mm und 1,3 mm — drei Größenordnungen
        // unter der Katastergenauigkeit. Die Schranke von 1 cm hält das fest,
        // ohne auf Fließkomma-Rauschen zu reagieren.
        const TOLERANZ_M: f64 = 0.01;
        for (name, lon, lat, crs, _, _) in REFERENZ {
            let (x, y) = wgs84_to_utm(*lon, *lat, *crs);
            let (lon2, lat2) = utm_to_wgs84(x, y, *crs);
            let dlon_m = (lon2 - lon).abs() * 111_320.0 * lat.to_radians().cos();
            let dlat_m = (lat2 - lat).abs() * 111_132.0;
            assert!(dlon_m < TOLERANZ_M, "{name}: Länge driftet um {dlon_m:.4} m");
            assert!(dlat_m < TOLERANZ_M, "{name}: Breite driftet um {dlat_m:.4} m");
        }
    }

    #[test]
    fn am_mittelmeridian_liegt_der_rechtswert_bei_500km() {
        let (x, _) = wgs84_to_utm(9.0, 51.0, Crs::Epsg25832);
        assert!((x - 500_000.0).abs() < 0.001, "x={x}");
        let (x33, _) = wgs84_to_utm(15.0, 51.0, Crs::Epsg25833);
        assert!((x33 - 500_000.0).abs() < 0.001, "x={x33}");
    }

    #[test]
    fn bbox_umrechnung_umschliesst_die_ecken() {
        let wgs = Bbox::new(9.178, 48.775, 9.180, 48.777);
        let utm = bbox_to_utm(wgs, Crs::Epsg25832);
        for (lon, lat) in [(9.178, 48.775), (9.180, 48.777), (9.179, 48.775)] {
            let (x, y) = wgs84_to_utm(lon, lat, Crs::Epsg25832);
            assert!(utm.contains(x, y), "({x:.1}, {y:.1}) liegt außerhalb");
        }
        // Rund 150 m Kantenlänge — plausibel für 0.002° auf dieser Breite.
        assert!(utm.width() > 100.0 && utm.width() < 250.0, "{:?}", utm.width());
    }

    #[test]
    fn ausgabekoordinaten_werden_auf_zentimeter_gerundet() {
        let mut g = crate::model::Geometry::from_polygon(vec![vec![[356_535.0, 5_645_138.0]]]);
        geometry_to_wgs84(&mut g, Crs::Epsg25832);
        let [lon, lat] = g.polygons[0][0][0];
        for v in [lon, lat] {
            let stellen = format!("{v}");
            let nach_komma = stellen.split_once('.').map(|(_, d)| d.len()).unwrap_or(0);
            assert!(nach_komma <= 7, "{v} hat {nach_komma} Nachkommastellen");
        }
        // Die Rundung darf höchstens gut einen Zentimeter verschieben.
        let (exakt_lon, exakt_lat) = utm_to_wgs84(356_535.0, 5_645_138.0, Crs::Epsg25832);
        assert!((lon - exakt_lon).abs() * 111_320.0 < 0.02);
        assert!((lat - exakt_lat).abs() * 111_132.0 < 0.02);
    }

    #[test]
    fn geometrie_wird_nach_wgs84_gedreht() {
        use crate::model::Geometry;
        let mut g = Geometry::from_polygon(vec![vec![
            [356_535.0, 5_645_138.0],
            [356_635.0, 5_645_138.0],
            [356_535.0, 5_645_238.0],
            [356_535.0, 5_645_138.0],
        ]]);
        geometry_to_wgs84(&mut g, Crs::Epsg25832);
        let [lon, lat] = g.polygons[0][0][0];
        assert!((lon - 6.958).abs() < 2e-5, "lon={lon}");
        assert!((lat - 50.940).abs() < 2e-5, "lat={lat}");
    }
}
