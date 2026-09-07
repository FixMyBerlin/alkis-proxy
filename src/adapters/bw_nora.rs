//! Baden-Württemberg, `nora:v_al_flurstueck`.
//!
//! Eigenes Schema mit vollständiger Attributierung. Zwei Eigenheiten:
//! Die Geometrie kommt als einzelnes `Polygon` (wird beim Parsen zu
//! MultiPolygon hochgestuft), und `flurstueckstext` enthält die
//! Flurstücksnummer (`"94/1"`) — keine Lagebezeichnung. Sie wird deshalb
//! nicht gemappt, da das Zielschema die Nummer ohnehin aus dem Kennzeichen bildet.

use crate::adapters::{geometry, iso_date, parcel_id, text, AdapterError};
use crate::config::StateKey;
use crate::model::Parcel;
use crate::parse::RawFeature;

pub fn map(raw: &RawFeature, state: StateKey) -> Result<Parcel, AdapterError> {
    let (id, source) = parcel_id(raw, &["flurstueckskennzeichen"])?;

    Ok(Parcel {
        parcel_id: id,
        parcel_id_source: source,
        bundesland: state.code(),
        gemarkung_name: text(raw, &["gemarkung_name"]),
        gemeinde_schluessel: text(raw, &["gemeinde_id"]),
        gemeinde_name: text(raw, &["gemeinde_name"]),
        kreis_name: None,
        lagebezeichnung: None,
        nutzung: None,
        flaeche_qm: raw.number(&["amtliche_flaeche"]),
        stand: iso_date(raw.field(&["beginn"])),
        geometry: geometry(raw)?,
    })
}
