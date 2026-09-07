//! Berlin, `alkis_flurstuecke:flurstuecke`.
//!
//! Zwei Fallstricke:
//!
//! * Das Kennzeichen `fsko` ist **18-stellig** — die Folgenummer fehlt. Der
//!   Kennzeichen-Parser füllt sie als unbelegt auf.
//! * `bezeich` enthält den Objekttyp (`"AX_Flurstueck"`), **keine**
//!   Lagebezeichnung. Das Feld wird bewusst nicht gemappt; andernfalls stünde
//!   bei jedem Berliner Flurstück derselbe technische String als Lage.

use crate::adapters::{geometry, iso_date, parcel_id, text, AdapterError};
use crate::config::StateKey;
use crate::model::Parcel;
use crate::parse::RawFeature;

pub fn map(raw: &RawFeature, state: StateKey) -> Result<Parcel, AdapterError> {
    let (id, source) = parcel_id(raw, &["fsko"])?;

    Ok(Parcel {
        parcel_id: id,
        parcel_id_source: source,
        bundesland: state.code(),
        gemarkung_name: text(raw, &["namgmk"]),
        gemeinde_schluessel: text(raw, &["gdz"]),
        gemeinde_name: text(raw, &["namgem"]),
        kreis_name: None,
        // bezeich absichtlich ausgelassen — enthält den Objekttyp.
        lagebezeichnung: None,
        nutzung: None,
        flaeche_qm: raw.number(&["afl"]),
        stand: iso_date(raw.field(&["beg", "zde"])),
        geometry: geometry(raw)?,
    })
}
