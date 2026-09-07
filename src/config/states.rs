//! Endpunkt-Katalog der 16 Bundesländer.
//!
//! Die Einträge wurden gegen die Dienste selbst verifiziert (`GetCapabilities`
//! und `DescribeFeatureType`). Bewusst *nicht* enthalten sind Flags für
//! EPSG:4326-Unterstützung oder BBOX-Achsenreihenfolge: Der Service fragt
//! grundsätzlich im nativen CRS des Landes an, wodurch beide Probleme entfallen.
//!
//! `bin/audit` prüft diesen Katalog gegen die Realität und meldet Drift.

/// Bundesland nach amtlichem Länderschlüssel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum StateKey {
    Sh,
    Hh,
    Ni,
    Hb,
    Nw,
    He,
    Rp,
    Bw,
    By,
    Sl,
    Be,
    Bb,
    Mv,
    Sn,
    St,
    Th,
}

impl StateKey {
    /// Kürzel, wie es in Feature-IDs und im `bundesland`-Property erscheint.
    pub fn code(self) -> &'static str {
        match self {
            StateKey::Sh => "SH",
            StateKey::Hh => "HH",
            StateKey::Ni => "NI",
            StateKey::Hb => "HB",
            StateKey::Nw => "NW",
            StateKey::He => "HE",
            StateKey::Rp => "RP",
            StateKey::Bw => "BW",
            StateKey::By => "BY",
            StateKey::Sl => "SL",
            StateKey::Be => "BE",
            StateKey::Bb => "BB",
            StateKey::Mv => "MV",
            StateKey::Sn => "SN",
            StateKey::St => "ST",
            StateKey::Th => "TH",
        }
    }

    pub fn from_code(code: &str) -> Option<Self> {
        Some(match code.to_ascii_uppercase().as_str() {
            "SH" => StateKey::Sh,
            "HH" => StateKey::Hh,
            "NI" => StateKey::Ni,
            "HB" => StateKey::Hb,
            "NW" => StateKey::Nw,
            "HE" => StateKey::He,
            "RP" => StateKey::Rp,
            "BW" => StateKey::Bw,
            "BY" => StateKey::By,
            "SL" => StateKey::Sl,
            "BE" => StateKey::Be,
            "BB" => StateKey::Bb,
            "MV" => StateKey::Mv,
            "SN" => StateKey::Sn,
            "ST" => StateKey::St,
            "TH" => StateKey::Th,
            _ => return None,
        })
    }
}

/// Quellschema. Bestimmt, welcher Adapter die Antwort abbildet.
///
/// Elf Länder liefern ein bit-identisches AdV-Schema („ALKIS vereinfacht"),
/// verifiziert für Brandenburg, NRW und Hessen. Nur drei Länder scheren aus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemaVariant {
    /// AdV „ALKIS vereinfacht", `ave:Flurstueck`, 24 Felder.
    Ave,
    /// INSPIRE `cp:CadastralParcel` — nur Kennzeichen, Label, Fläche, Geometrie.
    Inspire,
    /// Baden-Württemberg, `nora:v_al_flurstueck`.
    BwNora,
    /// Berlin, `alkis_flurstuecke:flurstuecke`.
    Berlin,
}

/// Natives Koordinatenreferenzsystem des Dienstes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Crs {
    /// ETRS89 / UTM Zone 32N — westliche Bundesländer.
    Epsg25832,
    /// ETRS89 / UTM Zone 33N — östliche Bundesländer.
    Epsg25833,
}

impl Crs {
    pub fn as_urn(self) -> &'static str {
        match self {
            Crs::Epsg25832 => "EPSG:25832",
            Crs::Epsg25833 => "EPSG:25833",
        }
    }
    pub fn code(self) -> u32 {
        match self {
            Crs::Epsg25832 => 25832,
            Crs::Epsg25833 => 25833,
        }
    }
}

/// Ausgabeformat, das beim Dienst angefordert wird.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    /// GML 3.2.1 — der Normalfall.
    Gml321,
    /// GeoJSON, sofern der Dienst es beherrscht (spart das XML-Parsen).
    GeoJson,
    /// Kein `OUTPUTFORMAT`-Parameter — der Dienst entscheidet selbst.
    ///
    /// Nötig für den Saarland-Endpunkt: Der dortige ArcGIS-Server lehnt jede
    /// explizite Formatangabe mit einer OGC-Exception ab, liefert ohne Angabe
    /// aber sauberes GML 3.2.
    ServerDefault,
}

impl OutputFormat {
    /// Wert für den `OUTPUTFORMAT`-Parameter.
    ///
    /// Kein `+` im MIME-Type: manche WFS-Server dekodieren es als Leerzeichen.
    pub fn as_param(self) -> Option<&'static str> {
        match self {
            OutputFormat::Gml321 => Some("text/xml; subtype=gml/3.2.1"),
            OutputFormat::GeoJson => Some("application/json"),
            OutputFormat::ServerDefault => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum License {
    /// Datenlizenz Deutschland – Namensnennung – 2.0 (Attribution erforderlich).
    DlDeBy20,
    /// Datenlizenz Deutschland – Zero – 2.0 (keine Attribution erforderlich).
    DlDeZero20,
    /// Creative Commons Namensnennung 4.0 (Attribution erforderlich).
    CcBy40,
}

impl License {
    pub fn url(self) -> &'static str {
        match self {
            License::DlDeBy20 => "https://www.govdata.de/dl-de/by-2-0",
            License::DlDeZero20 => "https://www.govdata.de/dl-de/zero-2-0",
            License::CcBy40 => "https://creativecommons.org/licenses/by/4.0/",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            License::DlDeBy20 => "DL-DE BY 2.0",
            License::DlDeZero20 => "DL-DE Zero 2.0",
            License::CcBy40 => "CC BY 4.0",
        }
    }
    /// Ob die Lizenz eine Namensnennung verlangt.
    pub fn requires_attribution(self) -> bool {
        matches!(self, License::DlDeBy20 | License::CcBy40)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Attribution {
    pub text: &'static str,
    pub url: &'static str,
    pub license: License,
}

#[derive(Debug, Clone, Copy)]
pub struct Endpoint {
    pub url: &'static str,
    pub typename: &'static str,
    pub schema: SchemaVariant,
    pub native_crs: Crs,
    pub output_format: OutputFormat,
}

#[derive(Debug, Clone, Copy)]
pub struct StateConfig {
    pub key: StateKey,
    pub label: &'static str,
    /// `None`, solange kein frei zugänglicher Endpunkt bekannt ist.
    pub endpoint: Option<Endpoint>,
    pub attribution: Option<Attribution>,
    /// Bekannte Eigenheit des Dienstes, für Betrieb und Diagnose.
    pub note: Option<&'static str>,
}

use Crs::{Epsg25832, Epsg25833};
use OutputFormat::{GeoJson, Gml321, ServerDefault};
use SchemaVariant::{Ave, Berlin, BwNora, Inspire};

/// Der vollständige Katalog. Reihenfolge nach amtlichem Länderschlüssel.
pub const STATES: &[StateConfig] = &[
    StateConfig {
        key: StateKey::Sh,
        label: "Schleswig-Holstein",
        endpoint: Some(Endpoint {
            url: "https://service.gdi-sh.de/SH_INSPIREDOWNLOAD_AI_CP_ALKIS",
            typename: "cp:CadastralParcel",
            schema: Inspire,
            native_crs: Epsg25832,
            output_format: Gml321,
        }),
        attribution: Some(Attribution {
            text: "© GeoBasis-DE/LVermGeo SH",
            url: "https://www.gdi-sh.de",
            license: License::CcBy40,
        }),
        note: Some("INSPIRE-Schema: liefert nur Kennzeichen, Label, Fläche, Geometrie."),
    },
    StateConfig {
        key: StateKey::Hh,
        label: "Hamburg",
        endpoint: Some(Endpoint {
            url: "https://geodienste.hamburg.de/WFS_HH_ALKIS_vereinfacht",
            typename: "ave:Flurstueck",
            schema: Ave,
            native_crs: Epsg25832,
            output_format: Gml321,
        }),
        attribution: Some(Attribution {
            text: "© LGV Hamburg",
            url: "https://www.geoportal-hamburg.de",
            license: License::DlDeBy20,
        }),
        note: None,
    },
    StateConfig {
        key: StateKey::Ni,
        label: "Niedersachsen",
        endpoint: Some(Endpoint {
            url: "https://opendata.lgln.niedersachsen.de/doorman/noauth/alkis_wfs_einfach",
            typename: "ave:Flurstueck",
            schema: Ave,
            native_crs: Epsg25832,
            output_format: Gml321,
        }),
        attribution: Some(Attribution {
            text: "© LGLN",
            url: "https://opendata.lgln.niedersachsen.de",
            license: License::CcBy40,
        }),
        note: None,
    },
    StateConfig {
        key: StateKey::Hb,
        label: "Bremen",
        endpoint: Some(Endpoint {
            url: "https://opendata.lgln.niedersachsen.de/doorman/noauth/alkishb_wfs_sf",
            typename: "adv:AX_Flurstueck",
            schema: Ave,
            native_crs: Epsg25832,
            output_format: Gml321,
        }),
        attribution: Some(Attribution {
            text: "© LGLN",
            url: "https://opendata.lgln.niedersachsen.de",
            license: License::CcBy40,
        }),
        note: Some("Läuft über den LGLN-Host und liefert das NAS-Schema (adv:AX_Flurstueck) mit abweichenden Feldnamen."),
    },
    StateConfig {
        key: StateKey::Nw,
        label: "Nordrhein-Westfalen",
        endpoint: Some(Endpoint {
            url: "https://www.wfs.nrw.de/geobasis/wfs_nw_alkis_vereinfacht",
            typename: "ave:Flurstueck",
            schema: Ave,
            native_crs: Epsg25832,
            output_format: Gml321,
        }),
        attribution: Some(Attribution {
            text: "© Geobasis-DE/NRW",
            url: "https://www.geoportal.nrw",
            license: License::DlDeZero20,
        }),
        note: None,
    },
    StateConfig {
        key: StateKey::He,
        label: "Hessen",
        endpoint: Some(Endpoint {
            url: "https://www.gds.hessen.de/wfs2/aaa-suite/cgi-bin/alkis/vereinf/wfs",
            typename: "ave:Flurstueck",
            schema: Ave,
            native_crs: Epsg25832,
            output_format: Gml321,
        }),
        attribution: Some(Attribution {
            text: "© GeoBasis-DE/HVBG",
            url: "https://gds.hessen.de",
            license: License::DlDeZero20,
        }),
        note: None,
    },
    StateConfig {
        key: StateKey::Rp,
        label: "Rheinland-Pfalz",
        endpoint: Some(Endpoint {
            url: "https://www.geoportal.rlp.de/registry/wfs/519",
            typename: "ave:Flurstueck",
            schema: Ave,
            native_crs: Epsg25832,
            // Der Dienst bewirbt JSON nicht und lehnt es mit einer OGC-Exception ab.
            output_format: Gml321,
        }),
        attribution: Some(Attribution {
            text: "© GeoBasis-DE/LVermGeoRP",
            url: "https://www.lvermgeo.rlp.de/geodaten-geoshop/open-data",
            license: License::DlDeBy20,
        }),
        note: None,
    },
    StateConfig {
        key: StateKey::Bw,
        label: "Baden-Württemberg",
        endpoint: Some(Endpoint {
            url: "https://owsproxy.lgl-bw.de/owsproxy/wfs/WFS_LGL-BW_ALKIS",
            typename: "nora:v_al_flurstueck",
            schema: BwNora,
            native_crs: Epsg25832,
            output_format: GeoJson,
        }),
        attribution: Some(Attribution {
            text: "© LGL BW",
            url: "https://www.lgl-bw.de",
            license: License::DlDeBy20,
        }),
        note: Some("Eigenes nora-Schema; liefert Polygon statt MultiPolygon."),
    },
    StateConfig {
        key: StateKey::By,
        label: "Bayern",
        endpoint: None,
        attribution: None,
        note: Some(
            "Kein verifizierter kostenfreier Endpunkt. GeodatenOnline-WFS ist gebührenpflichtig; \
             der INSPIRE-Downloaddienst ist der aussichtsreichste Kandidat und muss geprüft werden.",
        ),
    },
    StateConfig {
        key: StateKey::Sl,
        label: "Saarland",
        endpoint: Some(Endpoint {
            url: "https://geoportal.saarland.de/registry/wfs/414",
            typename: "ALKIS_ALKIS_WFS_ohne_Eig:Flurstueck",
            // ArcGIS-Dienst, liefert das AVE-Feldschema in Großbuchstaben.
            schema: Ave,
            native_crs: Epsg25832,
            output_format: ServerDefault,
        }),
        attribution: None,
        note: Some(
            "ArcGIS-Server: lehnt explizite OUTPUTFORMAT-Angaben ab und schreibt \
             die Feldnamen groß. Zeigt zeitweise 5xx; Circuit Breaker greift.",
        ),
    },
    StateConfig {
        key: StateKey::Be,
        label: "Berlin",
        endpoint: Some(Endpoint {
            url: "https://gdi.berlin.de/services/wfs/alkis_flurstuecke",
            typename: "alkis_flurstuecke:flurstuecke",
            schema: Berlin,
            native_crs: Epsg25833,
            output_format: GeoJson,
        }),
        attribution: Some(Attribution {
            text: "© GeoBasis-DE/Berlin",
            url: "https://www.berlin.de/sen/sbw/stadtdaten/geoportal/liegenschaftskataster/",
            license: License::DlDeZero20,
        }),
        note: Some("Kennzeichen 18-stellig ohne Folgenummer; bezeich enthält den Objekttyp, keine Lagebezeichnung."),
    },
    StateConfig {
        key: StateKey::Bb,
        label: "Brandenburg",
        endpoint: Some(Endpoint {
            url: "https://isk.geobasis-bb.de/ows/alkis_vereinf_wfs",
            typename: "ave:Flurstueck",
            schema: Ave,
            native_crs: Epsg25833,
            output_format: Gml321,
        }),
        attribution: Some(Attribution {
            text: "© GeoBasis-DE/LGB",
            url: "https://geoportal.brandenburg.de",
            license: License::DlDeBy20,
        }),
        note: None,
    },
    StateConfig {
        key: StateKey::Mv,
        label: "Mecklenburg-Vorpommern",
        endpoint: Some(Endpoint {
            url: "https://www.geodaten-mv.de/dienste/alkis_wfs_einfach",
            typename: "ave:Flurstueck",
            schema: Ave,
            native_crs: Epsg25833,
            output_format: Gml321,
        }),
        attribution: Some(Attribution {
            text: "© GeoBasis-DE/M-V",
            url: "https://www.laiv-mv.de/Geoinformation/",
            license: License::CcBy40,
        }),
        note: Some("Bietet kein EPSG:4326 an — nur über den nativen Pfad erreichbar."),
    },
    StateConfig {
        key: StateKey::Sn,
        label: "Sachsen",
        endpoint: Some(Endpoint {
            url: "https://geodienste.sachsen.de/aaa/public_alkis/vereinf/wfs",
            typename: "ave:Flurstueck",
            schema: Ave,
            native_crs: Epsg25833,
            output_format: Gml321,
        }),
        attribution: Some(Attribution {
            text: "© GeoSN",
            url: "https://www.landesvermessung.sachsen.de",
            license: License::DlDeBy20,
        }),
        note: Some("Bietet kein EPSG:4326 an — nur über den nativen Pfad erreichbar."),
    },
    StateConfig {
        key: StateKey::St,
        label: "Sachsen-Anhalt",
        endpoint: Some(Endpoint {
            url: "https://www.geodatenportal.sachsen-anhalt.de/wss/service/ST_LVermGeo_ALKIS_WFS_OpenData/guest",
            typename: "ave:Flurstueck",
            schema: Ave,
            native_crs: Epsg25832,
            output_format: Gml321,
        }),
        attribution: Some(Attribution {
            text: "© GeoBasis-DE/LVermGeo ST",
            url: "https://www.lvermgeo.sachsen-anhalt.de/de/gdp-open-data.html",
            license: License::DlDeBy20,
        }),
        note: None,
    },
    StateConfig {
        key: StateKey::Th,
        label: "Thüringen",
        endpoint: Some(Endpoint {
            url: "https://www.geoproxy.geoportal-th.de/geoproxy/services/adv_alkis_wfs",
            typename: "ave:Flurstueck",
            schema: Ave,
            native_crs: Epsg25832,
            output_format: Gml321,
        }),
        attribution: Some(Attribution {
            text: "© GDI-Th",
            url: "https://geoportal.thueringen.de",
            license: License::DlDeBy20,
        }),
        note: Some("Bietet kein EPSG:4326 an — nur über den nativen Pfad erreichbar."),
    },
];

pub fn all() -> &'static [StateConfig] {
    STATES
}

pub fn get(key: StateKey) -> &'static StateConfig {
    STATES
        .iter()
        .find(|s| s.key == key)
        .expect("Katalog enthält jedes Bundesland")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn katalog_enthaelt_alle_sechzehn_laender() {
        assert_eq!(STATES.len(), 16);
        let mut keys: Vec<_> = STATES.iter().map(|s| s.key).collect();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), 16, "Bundesländer dürfen sich nicht wiederholen");
    }

    #[test]
    fn code_und_from_code_sind_invers() {
        for s in STATES {
            assert_eq!(StateKey::from_code(s.key.code()), Some(s.key));
        }
        assert_eq!(StateKey::from_code("nw"), Some(StateKey::Nw));
        assert_eq!(StateKey::from_code("XX"), None);
    }

    #[test]
    fn ave_deckt_die_mehrheit_ab() {
        let ave = STATES
            .iter()
            .filter(|s| matches!(s.endpoint, Some(e) if e.schema == SchemaVariant::Ave))
            .count();
        assert!(ave >= 10, "erwartet mindestens 10 AVE-Länder, gefunden {ave}");
    }

    #[test]
    fn nur_bayern_ohne_endpunkt() {
        let ohne: Vec<_> = STATES
            .iter()
            .filter(|s| s.endpoint.is_none())
            .map(|s| s.key)
            .collect();
        assert_eq!(ohne, vec![StateKey::By]);
    }

    #[test]
    fn jeder_endpunkt_ist_https_und_hat_typename() {
        for s in STATES {
            if let Some(e) = s.endpoint {
                assert!(e.url.starts_with("https://"), "{}: {}", s.label, e.url);
                assert!(!e.typename.is_empty(), "{}", s.label);
            }
        }
    }

    #[test]
    fn attribution_vorhanden_wo_lizenz_sie_verlangt() {
        for s in STATES {
            if let Some(a) = s.attribution {
                if a.license.requires_attribution() {
                    assert!(!a.text.is_empty(), "{} braucht einen Attributionstext", s.label);
                }
            }
        }
    }

    #[test]
    fn oestliche_laender_nutzen_zone_33() {
        for key in [StateKey::Be, StateKey::Bb, StateKey::Mv, StateKey::Sn] {
            let e = get(key).endpoint.expect("Endpunkt vorhanden");
            assert_eq!(e.native_crs, Crs::Epsg25833, "{}", get(key).label);
        }
    }
}
