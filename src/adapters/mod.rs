//! Abbildung der Quellschemata auf das harmonisierte Zielschema.
//!
//! Die Menge der Schemata ist geschlossen und die Mapper sind zustandslos,
//! deshalb genügt hier ein Dispatch über [`SchemaVariant`] statt eines
//! Trait-Objekts — das spart eine Indirektion und macht die Fallunterscheidung
//! an einer Stelle sichtbar.
//!
//! **Leitprinzip:** Strukturelle Angaben (Gemarkung, Flur, Zähler, Nenner) werden
//! grundsätzlich aus dem Flurstückskennzeichen abgeleitet, nie aus Einzelfeldern.
//! Grund ist die Praxis: Thüringen liefert weder `gemaschl` noch `flstnrzae`,
//! Berlin kennt keine Folgenummer, INSPIRE hat die Felder gar nicht. Das
//! Kennzeichen ist die einzige Angabe, die in allen Schemata vorhanden ist.
//! Aus den Feldern kommen nur Klarnamen und Zusatzangaben.

mod ave;
mod berlin;
mod bw_nora;
mod inspire;

use crate::config::{SchemaVariant, StateKey};
use crate::model::{Parcel, ParcelId, ParcelIdError};
use crate::parse::RawFeature;

#[derive(Debug, thiserror::Error)]
pub enum AdapterError {
    #[error("Feature enthält kein Flurstückskennzeichen (geprüfte Felder: {tried})")]
    NoParcelId { tried: String },
    #[error("Flurstückskennzeichen {raw:?} ist unbrauchbar: {source}")]
    BadParcelId {
        raw: String,
        #[source]
        source: ParcelIdError,
    },
    #[error("Feature hat keine Geometrie")]
    NoGeometry,
}

/// Bildet ein Rohfeature auf das Zielschema ab.
pub fn map(
    schema: SchemaVariant,
    raw: &RawFeature,
    state: StateKey,
) -> Result<Parcel, AdapterError> {
    match schema {
        SchemaVariant::Ave => ave::map(raw, state),
        SchemaVariant::Inspire => inspire::map(raw, state),
        SchemaVariant::BwNora => bw_nora::map(raw, state),
        SchemaVariant::Berlin => berlin::map(raw, state),
    }
}

// --- gemeinsame Helfer für die Adapter ---

/// Sucht das Kennzeichen unter mehreren Feldnamen und parst es.
pub(crate) fn parcel_id(
    raw: &RawFeature,
    candidates: &[&'static str],
) -> Result<(ParcelId, &'static str), AdapterError> {
    for name in candidates {
        if let Some(value) = raw.field(&[name]) {
            let cleaned = strip_known_prefix(value);
            return ParcelId::parse(cleaned)
                .map(|id| (id, *name))
                .map_err(|source| AdapterError::BadParcelId {
                    raw: value.to_string(),
                    source,
                });
        }
    }
    Err(AdapterError::NoParcelId {
        tried: candidates.join(", "),
    })
}

/// INSPIRE-Bezeichner tragen einen Typpräfix (`CadastralParcel_0125…`) und
/// mitunter einen Namespace-URI davor. Beides gehört nicht zum Kennzeichen.
fn strip_known_prefix(value: &str) -> &str {
    let value = value.rsplit('/').next().unwrap_or(value);
    value
        .strip_prefix("CadastralParcel_")
        .unwrap_or(value)
}

/// Normalisiert Zeitangaben der Dienste auf ein reines ISO-Datum.
///
/// Beobachtete Formen: `2013-02-22Z` (NRW), `2018-11-16` (Berlin),
/// `2021-03-04T00:00:00` (Baden-Württemberg).
pub(crate) fn iso_date(value: Option<&str>) -> Option<String> {
    let v = value?.trim();
    if v.len() < 10 {
        return None;
    }
    let head = &v[..10];
    let looks_like_date = head.as_bytes().iter().enumerate().all(|(i, b)| {
        if i == 4 || i == 7 {
            *b == b'-'
        } else {
            b.is_ascii_digit()
        }
    });
    looks_like_date.then(|| head.to_string())
}

/// Geometrie übernehmen; ein Flurstück ohne Fläche ist für uns wertlos.
pub(crate) fn geometry(raw: &RawFeature) -> Result<crate::model::Geometry, AdapterError> {
    match &raw.geometry {
        Some(g) if !g.is_empty() => Ok(g.clone()),
        _ => Err(AdapterError::NoGeometry),
    }
}

pub(crate) fn text(raw: &RawFeature, names: &[&str]) -> Option<String> {
    raw.field(names).map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_date_normalisiert_die_beobachteten_formen() {
        assert_eq!(iso_date(Some("2013-02-22Z")).as_deref(), Some("2013-02-22"));
        assert_eq!(iso_date(Some("2018-11-16")).as_deref(), Some("2018-11-16"));
        assert_eq!(
            iso_date(Some("2021-03-04T00:00:00")).as_deref(),
            Some("2021-03-04")
        );
        assert_eq!(iso_date(Some("unbekannt")), None);
        assert_eq!(iso_date(Some("")), None);
        assert_eq!(iso_date(None), None);
    }

    #[test]
    fn inspire_praefixe_werden_entfernt() {
        assert_eq!(
            strip_known_prefix("CadastralParcel_01253801600432______"),
            "01253801600432______"
        );
        assert_eq!(
            strip_known_prefix(
                "https://registry.gdi-de.org/id/de.sh.inspire.cp.alkis/CadastralParcel_0125380160043______"
            ),
            "0125380160043______"
        );
        assert_eq!(strip_known_prefix("05495803101089______"), "05495803101089______");
    }
}
