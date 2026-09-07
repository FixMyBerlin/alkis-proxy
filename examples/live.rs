//! Prüft alle konfigurierten Landesdienste gegen die Realität.
//! Aufruf: `cargo run --example live`
//!
//! Vorgriff auf das Audit-Binary: zeigt je Bundesland, ob der Endpunkt
//! erreichbar ist, ob Features ankommen und ob sie sich abbilden lassen.

use std::time::{Duration, Instant};

use alkis_proxy::config::states;
use alkis_proxy::crs::reproject::{wgs84_to_utm, Bbox};
use alkis_proxy::upstream::UpstreamClient;

/// Ein Punkt in bebautem Gebiet je Bundesland (Länge, Breite).
const TESTPUNKTE: &[(&str, f64, f64)] = &[
    ("SH", 10.135, 54.322),
    ("HH", 9.993, 53.551),
    ("NI", 9.732, 52.375),
    ("HB", 8.807, 53.075),
    ("NW", 6.958, 50.940),
    ("HE", 8.240, 50.080),
    ("RP", 8.240, 49.992),
    ("BW", 9.178, 48.776),
    ("BY", 11.576, 48.137),
    ("SL", 6.996, 49.234),
    ("BE", 13.400, 52.510),
    ("BB", 13.060, 52.390),
    ("MV", 11.420, 53.630),
    ("SN", 13.740, 51.050),
    ("ST", 11.628, 52.121),
    ("TH", 11.030, 50.980),
];

#[tokio::main]
async fn main() {
    // Ohne Subscriber bleiben die Warnungen aus dem Mapping unsichtbar.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "warn".into()),
        )
        .with_target(false)
        .init();

    let client = UpstreamClient::new(Duration::from_secs(45)).expect("Client");

    println!(
        "{:<4} {:<24} {:>7} {:>6}  {}",
        "Land", "Dienst", "Feat.", "ms", "Erstes Kennzeichen / Fehler"
    );
    println!("{}", "-".repeat(100));

    let mut ok = 0;
    let mut fehler = 0;

    for state in states::all() {
        let code = state.key.code();
        let Some(endpoint) = state.endpoint else {
            println!("{code:<4} {:<24} {:>7} {:>6}  kein Endpunkt konfiguriert", "–", "–", "–");
            continue;
        };

        let (lon, lat) = TESTPUNKTE
            .iter()
            .find(|(c, _, _)| *c == code)
            .map(|(_, lon, lat)| (*lon, *lat))
            .expect("Testpunkt vorhanden");

        // 250-m-Umkreis im nativen CRS.
        let (x, y) = wgs84_to_utm(lon, lat, endpoint.native_crs);
        let bbox = Bbox::new(x - 250.0, y - 250.0, x + 250.0, y + 250.0);

        let host = endpoint
            .url
            .split('/')
            .nth(2)
            .unwrap_or(endpoint.url)
            .chars()
            .take(24)
            .collect::<String>();

        let start = Instant::now();
        let result = client.fetch(state, bbox, 20).await;
        let ms = start.elapsed().as_millis();

        match result {
            Ok(r) if !r.parcels.is_empty() => {
                ok += 1;
                let p = &r.parcels[0];
                let coord = p.geometry.polygons[0][0][0];
                println!(
                    "{code:<4} {host:<24} {:>7} {ms:>6}  {}  ({:.4}, {:.4})",
                    r.parcels.len(),
                    p.parcel_id.canonical(),
                    coord[0],
                    coord[1],
                );
            }
            Ok(_) => {
                println!("{code:<4} {host:<24} {:>7} {ms:>6}  leer (Testpunkt trifft kein Flurstück?)", 0);
            }
            Err(e) => {
                fehler += 1;
                let msg: String = e.to_string().chars().take(58).collect();
                println!("{code:<4} {host:<24} {:>7} {ms:>6}  {msg}", "–");
            }
        }
    }

    println!("\n{ok} Länder liefern Flurstücke, {fehler} mit Fehler.");
}
