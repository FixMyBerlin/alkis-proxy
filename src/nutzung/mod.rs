//! Amtliche tatsächliche Nutzung (`nutzungALKIS`) je Flurstück.
//!
//! Ergänzt das ohnehin vorhandene Feld `nutzung` (ein einzelner Wert direkt
//! aus der Flurstücksantwort, siehe `adapters::ave`) um eine zweite,
//! eigenständig abgerufene Ebene, die auch mehrere gleichzeitig geltende
//! Nutzungsarten abbilden kann — semikolongetrennt.
//!
//! Recherche gegen die Landesdienste selbst (`GetCapabilities`/
//! `DescribeFeatureType`, 2026-09-10) ergab zwei technisch grundverschiedene
//! Fälle, siehe [`join`] und [`intersect`]. Acht Länder unterstützen keinen
//! der beiden: Bayern hat gar keinen Endpunkt, Schleswig-Holstein (INSPIRE)
//! und Berlin (eigenes Schema) bieten über ihren offenen WFS keine
//! Nutzungsebene an, Bremen liefert das volle NAS-Schema mit rund zwanzig
//! einzelnen Objektarten statt einer Sammelklasse — außerhalb des Rahmens
//! dieser beiden Fälle.
//!
//! Beide Fälle geben ihre Werte durch [`catalog::canonical`], bevor sie in
//! `nutzungALKIS` landen: Die Länder schreiben denselben amtlichen Katalog
//! streckenweise unterschiedlich (Thüringen transliteriert und verstümmelt,
//! Saarland transliteriert teilweise) — die API liefert überall dieselbe
//! Schreibweise.

pub mod catalog;
pub mod geom;
pub mod intersect;
pub mod join;

use crate::config::{NutzungMode, SchemaVariant, StateConfig};
use crate::parse::RawFeature;

/// Ergebnis der Nutzungsermittlung für eine Kachel — hält entweder gar
/// nichts, den Join-Index oder den Verschneidungs-Index vor, je nachdem, was
/// `StateConfig::nutzung` für das Land vorsieht.
pub enum NutzungIndex {
    None,
    Join(join::JoinIndex),
    Intersect(intersect::IntersectIndex),
}

impl NutzungIndex {
    /// Baut den passenden Index aus den Rohfeatures der Nutzungsebene. `raws`
    /// ist leer (und der Index entsprechend wirkungslos), wenn der Dienst
    /// dafür nicht erreichbar war — siehe `UpstreamClient::fetch`.
    pub fn build(state: &StateConfig, schema: SchemaVariant, raws: Vec<RawFeature>) -> Self {
        let Some(cfg) = state.nutzung else { return NutzungIndex::None };
        match cfg.mode {
            NutzungMode::Join => NutzungIndex::Join(join::JoinIndex::build(&raws)),
            NutzungMode::Intersect => {
                NutzungIndex::Intersect(intersect::IntersectIndex::build(&raws, schema))
            }
        }
    }

    /// `nutzungALKIS` für das Flurstück, dessen Rohfeature (natives CRS, vor
    /// der Reprojektion nach WGS84) übergeben wird.
    pub fn nutzung_alkis(&self, raw: &RawFeature) -> Option<String> {
        match self {
            NutzungIndex::None => None,
            NutzungIndex::Join(index) => index.lookup(raw),
            NutzungIndex::Intersect(index) => index.lookup(raw.geometry.as_ref()?),
        }
    }
}
