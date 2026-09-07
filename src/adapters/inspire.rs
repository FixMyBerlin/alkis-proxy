//! INSPIRE `cp:CadastralParcel` — Schleswig-Holstein, Saarland und
//! voraussichtlich Bayern.
//!
//! Das ärmste der vier Schemata: Es kennt nur Kennzeichen, Label, Fläche und
//! Geometrie. Gemarkung, Flur, Zähler und Nenner existieren nicht als Felder —
//! sie werden vollständig aus dem Kennzeichen rekonstruiert, was der
//! Kennzeichen-Parser übernimmt. Genau dafür wurde er gebaut.

use crate::adapters::{geometry, iso_date, parcel_id, text, AdapterError};
use crate::config::StateKey;
use crate::model::Parcel;
use crate::parse::RawFeature;

pub fn map(raw: &RawFeature, state: StateKey) -> Result<Parcel, AdapterError> {
    // nationalCadastralReference ist der Normalfall; localId aus dem
    // inspireId-Block dient als Rückfallebene.
    let (id, source) = parcel_id(raw, &["nationalcadastralreference", "localid"])?;

    Ok(Parcel {
        parcel_id: id,
        parcel_id_source: source,
        bundesland: state.code(),
        // Klarnamen liefert INSPIRE nicht.
        gemarkung_name: None,
        gemeinde_schluessel: None,
        gemeinde_name: None,
        kreis_name: None,
        lagebezeichnung: text(raw, &["label"]),
        nutzung: None,
        flaeche_qm: raw.number(&["areavalue"]),
        stand: iso_date(raw.field(&["beginlifespanversion", "validfrom"])),
        geometry: geometry(raw)?,
    })
}
