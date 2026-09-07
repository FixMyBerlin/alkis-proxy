//! Aufbau der GetFeature-Anfrage.
//!
//! Der Dienst fragt grundsätzlich im **nativen CRS** des jeweiligen Landes an.
//! Das ist die zentrale Designentscheidung: In EPSG:4326 ist die
//! Achsenreihenfolge unter WFS 2.0 uneinheitlich implementiert (manche Server
//! erwarten `lon,lat`, andere `lat,lon`), und drei Länder — Mecklenburg-Vorpommern,
//! Sachsen und Thüringen — bieten 4326 überhaupt nicht an. Bei projizierten CRS
//! ist die Reihenfolge dagegen eindeutig East/North. Dadurch entfällt jede
//! landesspezifische Sonderbehandlung, und alle Länder sind erreichbar.

use crate::config::{Crs, Endpoint};
use crate::crs::reproject::Bbox;

#[derive(Debug, thiserror::Error)]
pub enum UpstreamError {
    #[error("{state}: Zeitüberschreitung nach {seconds} s")]
    Timeout { state: &'static str, seconds: u64 },
    #[error("{state}: Netzwerkfehler — {message}")]
    Network { state: &'static str, message: String },
    #[error("{state}: HTTP {status}")]
    HttpStatus { state: &'static str, status: u16 },
    #[error("{state}: Antwort nicht auswertbar — {message}")]
    Unparsable { state: &'static str, message: String },
    #[error("{state}: kein Endpunkt konfiguriert")]
    NoEndpoint { state: &'static str },
}

impl UpstreamError {
    /// Ob der Fehler auf ein Problem beim Anbieter hindeutet und damit auf den
    /// Circuit Breaker einzahlen soll. Parse-Fehler tun das nicht — sie deuten
    /// auf Schema-Drift hin, die ein erneuter Versuch nicht behebt.
    pub fn counts_towards_breaker(&self) -> bool {
        matches!(
            self,
            UpstreamError::Timeout { .. }
                | UpstreamError::Network { .. }
                | UpstreamError::HttpStatus { .. }
        )
    }
}

/// Baut die GetFeature-URL für eine Kachel.
pub fn build_get_feature_url(endpoint: &Endpoint, bbox: Bbox, count: usize) -> String {
    let crs = endpoint.native_crs;
    let sep = if endpoint.url.contains('?') { '&' } else { '?' };

    // Die BBOX trägt das CRS als fünftes Element — so schreibt es WFS 2.0 vor.
    // Bei UTM ist die Reihenfolge eindeutig minx,miny,maxx,maxy.
    let bbox_param = format!(
        "{:.3},{:.3},{:.3},{:.3},{}",
        bbox.min_x,
        bbox.min_y,
        bbox.max_x,
        bbox.max_y,
        crs_urn(crs)
    );

    let mut url = format!(
        "{}{}SERVICE=WFS&VERSION=2.0.0&REQUEST=GetFeature\
         &TYPENAMES={}&SRSNAME={}&BBOX={}&COUNT={}",
        endpoint.url,
        sep,
        urlencode(endpoint.typename),
        urlencode(crs_urn(crs)),
        urlencode(&bbox_param),
        count,
    );
    if let Some(fmt) = endpoint.output_format.as_param() {
        url.push_str("&OUTPUTFORMAT=");
        url.push_str(&urlencode(fmt));
    }
    url
}

fn crs_urn(crs: Crs) -> &'static str {
    crs.as_urn()
}

/// Minimales Prozent-Encoding für Query-Werte.
///
/// Reicht hier aus, weil ausschließlich kontrollierte Werte aus dem
/// Endpunkt-Katalog kodiert werden — keine Nutzereingaben.
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b',' | b':' => {
                out.push(b as char)
            }
            b' ' => out.push_str("%20"),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{states, StateKey};

    fn endpoint(key: StateKey) -> Endpoint {
        states::get(key).endpoint.expect("Endpunkt")
    }

    #[test]
    fn url_fragt_im_nativen_crs_an() {
        let bbox = Bbox::new(356_000.0, 5_645_000.0, 357_000.0, 5_646_000.0);
        let url = build_get_feature_url(&endpoint(StateKey::Nw), bbox, 5000);
        // ":" und "," sind in Query-Strings zulässig (RFC 3986) und werden von
        // allen geprüften Landesdiensten unkodiert akzeptiert.
        assert!(url.contains("SRSNAME=EPSG:25832"), "{url}");
        assert!(url.contains("BBOX=356000.000,5645000.000,357000.000,5646000.000,EPSG:25832"), "{url}");
        // Kein EPSG:4326 — genau darum geht es.
        assert!(!url.contains("4326"), "{url}");
    }

    #[test]
    fn oestliche_laender_nutzen_zone_33() {
        let bbox = Bbox::new(390_000.0, 5_820_000.0, 391_000.0, 5_821_000.0);
        let url = build_get_feature_url(&endpoint(StateKey::Be), bbox, 100);
        assert!(url.contains("25833"), "{url}");
    }

    #[test]
    fn wfs2_parameter_sind_vollstaendig() {
        let bbox = Bbox::new(0.0, 0.0, 1.0, 1.0);
        let url = build_get_feature_url(&endpoint(StateKey::He), bbox, 42);
        for expected in [
            "SERVICE=WFS",
            "VERSION=2.0.0",
            "REQUEST=GetFeature",
            "TYPENAMES=",
            "COUNT=42",
            "OUTPUTFORMAT=",
        ] {
            assert!(url.contains(expected), "{expected} fehlt in {url}");
        }
    }

    #[test]
    fn bestehende_query_parameter_bleiben_erhalten() {
        // Rheinland-Pfalz und Saarland liefern Endpunkte mit Pfad-IDs; falls
        // ein Endpunkt bereits eine Query trägt, darf sie nicht zerstört werden.
        let ep = Endpoint {
            url: "https://example.org/wfs?token=abc",
            ..endpoint(StateKey::Nw)
        };
        let url = build_get_feature_url(&ep, Bbox::new(0.0, 0.0, 1.0, 1.0), 10);
        assert!(url.contains("token=abc&SERVICE=WFS"), "{url}");
    }

    #[test]
    fn outputformat_wird_kodiert_ohne_plus() {
        // "+" im MIME-Type wird von manchen Servern als Leerzeichen gelesen.
        let url = build_get_feature_url(&endpoint(StateKey::Nw), Bbox::new(0.0, 0.0, 1.0, 1.0), 1);
        assert!(url.contains("gml%2F3.2.1"), "{url}");
        assert!(!url.contains('+'), "{url}");
    }

    #[test]
    fn serverdefault_setzt_keinen_outputformat_parameter() {
        let ep = Endpoint {
            output_format: crate::config::OutputFormat::ServerDefault,
            ..endpoint(StateKey::Nw)
        };
        let url = build_get_feature_url(&ep, Bbox::new(0.0, 0.0, 1.0, 1.0), 1);
        assert!(!url.contains("OUTPUTFORMAT"), "{url}");
        assert!(url.contains("SERVICE=WFS"), "{url}");
    }

    #[test]
    fn geojson_laender_fordern_json_an() {
        let url = build_get_feature_url(&endpoint(StateKey::Bw), Bbox::new(0.0, 0.0, 1.0, 1.0), 1);
        assert!(url.contains("application%2Fjson"), "{url}");
    }

    #[test]
    fn parse_fehler_zaehlen_nicht_auf_den_breaker() {
        let parse = UpstreamError::Unparsable {
            state: "NW",
            message: "x".into(),
        };
        assert!(!parse.counts_towards_breaker());
        let timeout = UpstreamError::Timeout {
            state: "NW",
            seconds: 30,
        };
        assert!(timeout.counts_towards_breaker());
    }
}
