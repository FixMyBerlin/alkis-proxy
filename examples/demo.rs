//! Zeigt die Harmonisierung an echten Antworten aller vier Quellschemata.
//! Aufruf: `cargo run --example demo`

use alkis_proxy::adapters;
use alkis_proxy::config::{SchemaVariant, StateKey};
use alkis_proxy::parse::{parse_geojson, parse_gml};

fn main() {
    let gml: &[(&str, &str, SchemaVariant, StateKey)] = &[
        ("NW", include_str!("../tests/fixtures/ave_nw.gml"), SchemaVariant::Ave, StateKey::Nw),
        ("TH", include_str!("../tests/fixtures/ave_th.gml"), SchemaVariant::Ave, StateKey::Th),
        ("SH", include_str!("../tests/fixtures/inspire_sh.gml"), SchemaVariant::Inspire, StateKey::Sh),
    ];
    let json: &[(&str, &str, SchemaVariant, StateKey)] = &[
        ("BE", include_str!("../tests/fixtures/berlin.json"), SchemaVariant::Berlin, StateKey::Be),
        ("BW", include_str!("../tests/fixtures/bw_nora.json"), SchemaVariant::BwNora, StateKey::Bw),
    ];

    println!(
        "{:<4} {:<22} {:<8} {:<7} {:<10} {:<18} {:>10}",
        "Land", "parcelId", "Gemark.", "Flur", "Nummer", "Gemarkungsname", "Fläche m²"
    );
    println!("{}", "-".repeat(88));

    for (code, body, schema, state) in gml.iter().chain(json.iter()) {
        let raws = if body.trim_start().starts_with('<') {
            parse_gml(body).expect("GML")
        } else {
            parse_geojson(body).expect("GeoJSON")
        };
        for raw in &raws {
            let p = adapters::map(*schema, raw, *state).expect("Mapping");
            println!(
                "{:<4} {:<22} {:<8} {:<7} {:<10} {:<18} {:>10}",
                code,
                p.parcel_id.canonical(),
                p.parcel_id.gemarkung_schluessel(),
                p.parcel_id.flur().unwrap_or_else(|| "–".into()),
                p.parcel_id.flurstuecksnummer(),
                p.gemarkung_name.as_deref().unwrap_or("–"),
                p.flaeche_qm.map(|f| format!("{f:.0}")).unwrap_or_else(|| "–".into()),
            );
        }
    }

    // Ein vollständiges Feature als GeoJSON.
    let raws = parse_gml(include_str!("../tests/fixtures/ave_nw.gml")).unwrap();
    let p = adapters::map(SchemaVariant::Ave, &raws[0], StateKey::Nw).unwrap();
    let mut f = p.to_feature();
    f["geometry"]["coordinates"] = serde_json::json!("… gekürzt …");
    println!("\nBeispiel-Feature (NW):\n{}", serde_json::to_string_pretty(&f).unwrap());
}
