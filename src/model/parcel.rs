//! Das harmonisierte Zielschema.
//!
//! Alle Landesadapter bilden auf diesen Typ ab. Er ist bewusst flach und
//! vollständig: nicht belegbare Felder werden als `null` serialisiert statt
//! weggelassen, damit Clients über alle Bundesländer hinweg dieselbe Struktur
//! sehen.

use serde::Serialize;

use crate::model::ParcelId;

/// Ein Ring aus Positionen `[lon, lat]`.
pub type Ring = Vec<[f64; 2]>;
/// Ein Polygon: äußerer Ring, danach beliebig viele Löcher.
pub type PolygonRings = Vec<Ring>;

/// Geometrie eines Flurstücks. Immer `MultiPolygon` — Quellen, die einzelne
/// `Polygon`/`Surface`-Geometrien liefern (z. B. Baden-Württemberg), werden
/// beim Mapping hochgestuft, damit Clients keine Fallunterscheidung brauchen.
#[derive(Debug, Clone, PartialEq)]
pub struct Geometry {
    pub polygons: Vec<PolygonRings>,
}

impl Geometry {
    pub fn multi_polygon(polygons: Vec<PolygonRings>) -> Self {
        Geometry { polygons }
    }

    /// Stuft ein einzelnes Polygon zu einem MultiPolygon hoch.
    pub fn from_polygon(rings: PolygonRings) -> Self {
        Geometry {
            polygons: vec![rings],
        }
    }

    pub fn is_empty(&self) -> bool {
        self.polygons.iter().all(|p| p.iter().all(|r| r.is_empty()))
    }
}

impl Serialize for Geometry {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut st = s.serialize_struct("Geometry", 2)?;
        st.serialize_field("type", "MultiPolygon")?;
        st.serialize_field("coordinates", &self.polygons)?;
        st.end()
    }
}

/// Ein Flurstück im harmonisierten Zielschema.
#[derive(Debug, Clone)]
pub struct Parcel {
    pub parcel_id: ParcelId,
    /// Aus welchem Quellattribut das Kennzeichen stammt — für Diagnose bei
    /// Schema-Drift der Länder.
    pub parcel_id_source: &'static str,
    pub bundesland: &'static str,
    pub gemarkung_name: Option<String>,
    pub gemeinde_schluessel: Option<String>,
    pub gemeinde_name: Option<String>,
    pub kreis_name: Option<String>,
    pub lagebezeichnung: Option<String>,
    pub nutzung: Option<String>,
    /// Amtliche Fläche in Quadratmetern.
    pub flaeche_qm: Option<f64>,
    /// Stand der Daten als ISO-Datum, soweit die Quelle eines liefert.
    pub stand: Option<String>,
    pub geometry: Geometry,
}

/// Serialisierbare Sicht: die Properties eines GeoJSON-Features.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ParcelProperties<'a> {
    parcel_id: String,
    parcel_id_source: &'a str,
    bundesland: &'a str,
    gemarkung_schluessel: String,
    gemarkung_name: Option<&'a str>,
    flur: Option<String>,
    zaehler: String,
    nenner: Option<String>,
    flurstuecksnummer: String,
    flaeche_qm: Option<f64>,
    gemeinde_schluessel: Option<&'a str>,
    gemeinde_name: Option<&'a str>,
    kreis_name: Option<&'a str>,
    lagebezeichnung: Option<&'a str>,
    nutzung: Option<&'a str>,
    stand: Option<&'a str>,
}

/// Ein vollständiges GeoJSON-Feature.
#[derive(Debug, Serialize)]
struct Feature<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    id: String,
    geometry: &'a Geometry,
    properties: ParcelProperties<'a>,
}

impl Parcel {
    /// Stabile, bundesweit eindeutige Feature-ID: Bundeslandkürzel plus
    /// kanonisches Kennzeichen. Die Präfixierung verhindert Kollisionen
    /// zwischen Ländern, deren Kennzeichen sich theoretisch überschneiden.
    pub fn feature_id(&self) -> String {
        format!("{}:{}", self.bundesland, self.parcel_id.canonical())
    }

    /// Serialisiert das Flurstück als GeoJSON-Feature.
    pub fn to_feature(&self) -> serde_json::Value {
        let feature = Feature {
            kind: "Feature",
            id: self.feature_id(),
            geometry: &self.geometry,
            properties: ParcelProperties {
                parcel_id: self.parcel_id.canonical(),
                parcel_id_source: self.parcel_id_source,
                bundesland: self.bundesland,
                gemarkung_schluessel: self.parcel_id.gemarkung_schluessel(),
                gemarkung_name: self.gemarkung_name.as_deref(),
                flur: self.parcel_id.flur(),
                zaehler: self.parcel_id.zaehler(),
                nenner: self.parcel_id.nenner(),
                flurstuecksnummer: self.parcel_id.flurstuecksnummer(),
                flaeche_qm: self.flaeche_qm,
                gemeinde_schluessel: self.gemeinde_schluessel.as_deref(),
                gemeinde_name: self.gemeinde_name.as_deref(),
                kreis_name: self.kreis_name.as_deref(),
                lagebezeichnung: self.lagebezeichnung.as_deref(),
                nutzung: self.nutzung.as_deref(),
                stand: self.stand.as_deref(),
            },
        };
        serde_json::to_value(feature).expect("Parcel ist immer serialisierbar")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn beispiel() -> Parcel {
        Parcel {
            parcel_id: ParcelId::parse("08146000000094000100").unwrap(),
            parcel_id_source: "flurstueckskennzeichen",
            bundesland: "BW",
            gemarkung_name: Some("Stuttgart".into()),
            gemeinde_schluessel: Some("0811100".into()),
            gemeinde_name: Some("Stuttgart".into()),
            kreis_name: None,
            lagebezeichnung: None,
            nutzung: None,
            flaeche_qm: Some(632.0),
            stand: Some("2018-11-16".into()),
            geometry: Geometry::from_polygon(vec![vec![
                [9.1776968, 48.7769798],
                [9.1776437, 48.7769245],
                [9.1777193, 48.776952],
                [9.1776968, 48.7769798],
            ]]),
        }
    }

    #[test]
    fn feature_id_ist_bundeslandpraefixiert() {
        assert_eq!(beispiel().feature_id(), "BW:081460___000940001__");
    }

    #[test]
    fn geojson_struktur_stimmt() {
        let f = beispiel().to_feature();
        assert_eq!(f["type"], "Feature");
        assert_eq!(f["id"], "BW:081460___000940001__");
        assert_eq!(f["geometry"]["type"], "MultiPolygon");
        // Einzelnes Polygon wurde zu MultiPolygon hochgestuft: eine Ebene mehr.
        assert_eq!(f["geometry"]["coordinates"][0][0][0][0], 9.1776968);
    }

    #[test]
    fn properties_sind_camel_case_und_vollstaendig() {
        let f = beispiel().to_feature();
        let p = &f["properties"];
        assert_eq!(p["parcelId"], "081460___000940001__");
        assert_eq!(p["bundesland"], "BW");
        assert_eq!(p["gemarkungSchluessel"], "081460");
        assert_eq!(p["gemarkungName"], "Stuttgart");
        assert!(p["flur"].is_null(), "BW führt keine Fluren");
        assert_eq!(p["zaehler"], "94");
        assert_eq!(p["nenner"], "1");
        assert_eq!(p["flurstuecksnummer"], "94/1");
        assert_eq!(p["flaecheQm"], 632.0);
        // Nicht belegte Felder sind null, nicht abwesend.
        assert!(p.get("kreisName").is_some());
        assert!(p["kreisName"].is_null());
        assert!(p["lagebezeichnung"].is_null());
    }

    #[test]
    fn polygon_wird_zu_multipolygon_hochgestuft() {
        let g = Geometry::from_polygon(vec![vec![[1.0, 2.0]]]);
        assert_eq!(g.polygons.len(), 1);
        let v = serde_json::to_value(&g).unwrap();
        assert_eq!(v["type"], "MultiPolygon");
    }
}
