//! AdV-Schema „ALKIS vereinfacht" — elf Bundesländer.
//!
//! Für Brandenburg, NRW und Hessen ist das Schema per `DescribeFeatureType`
//! als bit-identisch verifiziert (24 Felder). Thüringen weicht jedoch ab: dort
//! fehlen `gemaschl`, `flurschl`, `kreisschl` und `flstnrzae`/`flstnrnen`,
//! stattdessen gibt es ein zusammengesetztes `flurstnr`. Alle Felder werden
//! deshalb als optional behandelt.
//!
//! Bremen fällt zusätzlich aus dem Rahmen: Es liefert über denselben Host wie
//! Niedersachsen das NAS-Schema (`adv:AX_Flurstueck`) mit eigenen Feldnamen —
//! `flurstueckskennzeichen` statt `flstkennz`, `amtlicheFlaeche` statt `flaeche`.
//! Die Semantik ist identisch, deshalb deckt dieser Adapter beide Benennungen
//! über Kandidatenlisten ab, statt sie in einem fast gleichen zweiten Adapter
//! zu duplizieren.

use crate::adapters::{geometry, iso_date, parcel_id, text, AdapterError};
use crate::config::StateKey;
use crate::model::Parcel;
use crate::parse::RawFeature;

pub fn map(raw: &RawFeature, state: StateKey) -> Result<Parcel, AdapterError> {
    let (id, source) = parcel_id(raw, &["flstkennz", "flurstueckskennzeichen"])?;

    Ok(Parcel {
        parcel_id: id,
        parcel_id_source: source,
        bundesland: state.code(),
        gemarkung_name: text(raw, &["gemarkung"]),
        gemeinde_schluessel: text(raw, &["gmdschl"]),
        gemeinde_name: text(raw, &["gemeinde"]),
        kreis_name: text(raw, &["kreis"]),
        lagebezeichnung: text(raw, &["lagebeztxt"]),
        nutzung: text(raw, &["tntxt"]),
        flaeche_qm: raw.number(&["flaeche", "amtlicheflaeche"]),
        stand: iso_date(raw.field(&["aktualit", "beginnt"])),
        geometry: geometry(raw)?,
    })
}
