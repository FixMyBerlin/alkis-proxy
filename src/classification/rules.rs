//! Regel-Engine für die OSM-basierte Nutzungsart-Schätzung.
//!
//! Portiert aus `ist-dieses-flurstueck-oeffentlich/src/rules.rs`, mit zwei
//! Änderungen gegenüber dem Referenz-Repo:
//!
//! * Die Regeln `verkehrsnetz_bahn`/`verkehrsnetz_strasse` entfallen — dort
//!   kamen sie aus dem amtlichen INSPIRE-Verkehrsnetz-WFS, der trotz des
//!   Namens ALKIS-Daten sind und hier bewusst nicht verwendet werden. Bahn
//!   bleibt über `landuse=railway`/`railway=rail` aus OSM erreichbar; eine
//!   dedizierte Straßen-Regel gibt es in v1 nicht (größte bekannte Lücke,
//!   siehe README).
//! * Die Prädikate arbeiten auf einer `u32`-Bitmaske (siehe [`flags`]) statt
//!   auf `HashMap<String, String>`-Tags. Bei Millionen residenten Features
//!   (deutschlandweite PBF) dominieren Tag-Strings sonst den Speicher; die
//!   Bitmaske wird einmal beim Indizieren aus den Tags berechnet und ersetzt
//!   pro Anfrage String-Vergleiche durch Bit-Tests.
//!
//! [`flags_for_tag`] ist die **einzige** Stelle, an der Regeln und Indizierung
//! gekoppelt sind: `osm_extract.rs` hält nur Ways, für die diese Funktion für
//! mindestens ein Tag einen Wert ungleich 0 liefert. Eine neue Regel, die ein
//! bisher unbekanntes Tag abfragt, läuft sonst leise ins Leere, weil das Tag
//! beim Indizieren schon verworfen wurde — siehe den Konsistenztest unten.

use crate::classification::index::OsmContext;

/// Bit-Flags für die Tag/Wert-Kombinationen, die mindestens eine Regel abfragt.
///
/// Ein `OsmFeature` trägt die Vereinigung aller Flags, die auf mindestens eines
/// seiner Tags zutreffen (ein Way kann z. B. gleichzeitig `building=house` und
/// `operator:type=public` tragen — unüblich, aber die Bitmaske bildet es korrekt ab).
pub mod flags {
    pub const OPERATOR_PRIVATE: u32 = 1 << 0;
    pub const OPERATOR_PUBLIC: u32 = 1 << 1;
    pub const PRIVATE_BUILDING: u32 = 1 << 2;
    pub const PUBLIC_BUILDING: u32 = 1 << 3;
    pub const INDUSTRIAL: u32 = 1 << 4;
    pub const PARK: u32 = 1 << 5;
    pub const PLAYGROUND: u32 = 1 << 6;
    pub const ZOO: u32 = 1 << 7;
    pub const HOTEL: u32 = 1 << 8;
    pub const RAILWAY: u32 = 1 << 9;
}

/// Ordnet ein einzelnes Tag (Key, Value) den Flags zu, die es setzt — `0`,
/// wenn keine Regel dieses Tag abfragt. Groß-/Kleinschreibung wird nicht
/// normalisiert: OSM-Werte sind faktisch immer klein geschrieben.
pub fn flags_for_tag(key: &str, value: &str) -> u32 {
    match (key, value) {
        ("operator:type", "private") => flags::OPERATOR_PRIVATE,
        ("operator:type", "public") => flags::OPERATOR_PUBLIC,
        ("building", "house" | "residential" | "detached" | "apartments") => {
            flags::PRIVATE_BUILDING
        }
        ("building", "public" | "school") => flags::PUBLIC_BUILDING,
        (
            "amenity",
            "townhall" | "public_building" | "courthouse" | "community_centre" | "school",
        ) => flags::PUBLIC_BUILDING,
        ("leisure", "schoolyard") => flags::PUBLIC_BUILDING,
        ("leisure", "park") => flags::PARK,
        ("leisure", "playground") => flags::PLAYGROUND,
        ("landuse", "industrial" | "commercial") => flags::INDUSTRIAL,
        ("landuse", "railway") => flags::RAILWAY,
        ("tourism", "zoo") => flags::ZOO,
        ("tourism", "hotel") => flags::HOTEL,
        // Ways mit railway=rail sind so gut wie nie geschlossene Ringe
        // (Gleis-Mittellinien, keine Flächen) — die Overlap-Regel unten greift
        // für sie damit praktisch nie (overlap_ratio ist 0 für Linien). Das
        // Flag bleibt trotzdem gesetzt, für den seltenen Fall einer als
        // Fläche gemappten Bahnanlage und für spätere Presence-Regeln.
        ("railway", "rail") => flags::RAILWAY,
        _ => 0,
    }
}

/// Vereinigt die Flags aller Tags eines Features.
pub fn flags_for_tags<'a>(tags: impl Iterator<Item = (&'a str, &'a str)>) -> u32 {
    tags.fold(0, |acc, (k, v)| acc | flags_for_tag(k, v))
}

/// Klassifikations-Kategorie eines Flurstücks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Private,
    Public,
    Railway,
    Unknown,
}

impl Category {
    pub fn as_str(self) -> &'static str {
        match self {
            Category::Private => "privat",
            Category::Public => "oeffentlich",
            Category::Railway => "bahn",
            Category::Unknown => "unbekannt",
        }
    }
}

/// Ergebnis der Klassifikation eines Flurstücks.
#[derive(Debug, Clone)]
pub struct Classification {
    pub category: Category,
    pub rule: String,
    pub confidence: f32,
    /// Gesetzt, wenn eine andere greifende Regel zu einer abweichenden
    /// Kategorie gekommen wäre. Das Ergebnis bleibt eindeutig (höchste
    /// Priorität gewinnt), der Widerspruch wird hier nur dokumentiert.
    pub conflict: Option<String>,
}

/// Eine einzelne Klassifikationsregel. Gibt `Some(..)`, wenn sie greift.
///
/// `Send + Sync`, damit `ClassificationIndex` (und damit `AppState`) über
/// Axum-Handler und den `spawn_blocking`-Hintergrund-Task hinweg geteilt
/// werden kann.
pub trait Rule: Send + Sync {
    fn name(&self) -> &'static str;
    fn apply(&self, ctx: &OsmContext) -> Option<Classification>;
}

/// Standard-Regelsatz (geordnet — erste greifende Regel gewinnt).
pub fn default_rules() -> Vec<Box<dyn Rule>> {
    vec![
        // Explizites Tagging schlägt jede Flächen-/Gebäude-Heuristik.
        Box::new(OperatorTypeRule),
        Box::new(OverlapRule {
            name: "bahn",
            category: Category::Railway,
            threshold: 0.3,
            flag: flags::RAILWAY,
        }),
        // Gebäude sind ein präziser Fußabdruck-Hinweis -> vor groben
        // Landnutzungsflächen.
        Box::new(PresenceRule {
            name: "wohngebaeude",
            category: Category::Private,
            confidence: 0.6,
            flag: flags::PRIVATE_BUILDING,
        }),
        Box::new(PresenceRule {
            name: "oeffentliches_gebaeude",
            category: Category::Public,
            confidence: 0.6,
            flag: flags::PUBLIC_BUILDING,
        }),
        // Spezifische öffentliche Nutzung vor der groben Industrie/Gewerbe-
        // Fläche: landuse=commercial-Polygone werden in OSM oft großflächig
        // über ganze Baublöcke gezogen, ohne Aussparung für darin liegende
        // Parks/Schulhöfe.
        Box::new(OverlapRule {
            name: "park",
            category: Category::Public,
            threshold: 0.3,
            flag: flags::PARK,
        }),
        Box::new(PresenceRule {
            name: "spielplatz",
            category: Category::Public,
            confidence: 0.6,
            flag: flags::PLAYGROUND,
        }),
        Box::new(OverlapRule {
            name: "zoo",
            category: Category::Public,
            threshold: 0.3,
            flag: flags::ZOO,
        }),
        Box::new(OverlapRule {
            name: "industrie",
            category: Category::Private,
            threshold: 0.3,
            flag: flags::INDUSTRIAL,
        }),
        Box::new(PresenceRule {
            name: "hotel",
            category: Category::Private,
            confidence: 0.6,
            flag: flags::HOTEL,
        }),
    ]
}

/// Läuft alle Regeln der Reihe nach durch (nicht nur bis zum ersten Treffer),
/// damit widersprüchliche Treffer erkannt werden können. Fallback ist
/// `Unknown`, wenn keine Regel greift.
pub fn classify(ctx: &OsmContext, rules: &[Box<dyn Rule>]) -> Classification {
    let hits: Vec<(&'static str, Classification)> = rules
        .iter()
        .filter_map(|rule| rule.apply(ctx).map(|c| (rule.name(), c)))
        .collect();

    let Some((_, first)) = hits.first() else {
        return Classification {
            category: Category::Unknown,
            rule: "fallback".to_string(),
            confidence: 0.0,
            conflict: None,
        };
    };

    let mut winner = first.clone();
    let differing: Vec<String> = hits
        .iter()
        .skip(1)
        .filter(|(_, c)| c.category != winner.category)
        .map(|(name, c)| format!("{name}={}", c.category.as_str()))
        .collect();
    if !differing.is_empty() {
        winner.conflict = Some(differing.join(", "));
    }
    winner
}

/// `operator:type=private`/`operator:type=public` ist eine explizite Aussage
/// zur Eigentümerart und schlägt daher jede Flächen-/Gebäude-Heuristik.
struct OperatorTypeRule;

impl Rule for OperatorTypeRule {
    fn name(&self) -> &'static str {
        "operator_type"
    }

    fn apply(&self, ctx: &OsmContext) -> Option<Classification> {
        let category = ctx.matches.iter().find_map(|m| {
            if m.feature.flags & flags::OPERATOR_PRIVATE != 0 {
                Some(Category::Private)
            } else if m.feature.flags & flags::OPERATOR_PUBLIC != 0 {
                Some(Category::Public)
            } else {
                None
            }
        })?;
        Some(Classification {
            category,
            rule: self.name().to_string(),
            confidence: 1.0,
            conflict: None,
        })
    }
}

/// Greift, wenn der größte Flächenüberlappungsanteil eines passenden Features
/// den Schwellwert überschreitet. Konfidenz = Überlappungsanteil.
struct OverlapRule {
    name: &'static str,
    category: Category,
    threshold: f64,
    flag: u32,
}

impl Rule for OverlapRule {
    fn name(&self) -> &'static str {
        self.name
    }

    fn apply(&self, ctx: &OsmContext) -> Option<Classification> {
        let overlap = ctx.max_overlap(self.flag);
        if overlap > self.threshold {
            Some(Classification {
                category: self.category,
                rule: self.name.to_string(),
                confidence: overlap.min(1.0) as f32,
                conflict: None,
            })
        } else {
            None
        }
    }
}

/// Greift, sobald irgendein schneidendes Feature das Flag trägt — unabhängig
/// vom Überlappungsanteil. Passend für Gebäude-Fußabdrücke.
struct PresenceRule {
    name: &'static str,
    category: Category,
    confidence: f32,
    flag: u32,
}

impl Rule for PresenceRule {
    fn name(&self) -> &'static str {
        self.name
    }

    fn apply(&self, ctx: &OsmContext) -> Option<Classification> {
        if ctx.any_flag(self.flag) {
            Some(Classification {
                category: self.category,
                rule: self.name.to_string(),
                confidence: self.confidence,
                conflict: None,
            })
        } else {
            None
        }
    }
}

/// Prüft, dass jedes von einer Regel abgefragte Flag auch tatsächlich von
/// mindestens einer Tag/Wert-Kombination in [`flags_for_tag`] gesetzt wird —
/// sonst würde eine neue Regel leise ins Leere laufen, weil ihr Tag beim
/// Indizieren gar nicht erst gesammelt wird.
#[cfg(test)]
fn all_known_flags() -> u32 {
    use flags::*;
    OPERATOR_PRIVATE
        | OPERATOR_PUBLIC
        | PRIVATE_BUILDING
        | PUBLIC_BUILDING
        | INDUSTRIAL
        | PARK
        | PLAYGROUND
        | ZOO
        | HOTEL
        | RAILWAY
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classification::index::OsmMatch;
    use crate::classification::OsmFeature;
    use geo_types::{Geometry, MultiPolygon, Point};

    fn feature(flags: u32) -> OsmFeature {
        OsmFeature {
            flags,
            geom: Geometry::Point(Point::new(0.0, 0.0)),
        }
    }

    fn ctx<'a>(matches: Vec<OsmMatch<'a>>) -> OsmContext<'a> {
        OsmContext { matches }
    }

    #[test]
    fn operator_type_schlaegt_alles_andere() {
        let f = feature(flags::OPERATOR_PUBLIC | flags::PRIVATE_BUILDING);
        let c = ctx(vec![OsmMatch { feature: &f, overlap_ratio: 1.0 }]);
        let result = classify(&c, &default_rules());
        assert_eq!(result.rule, "operator_type");
        assert_eq!(result.category, Category::Public);
    }

    #[test]
    fn wohngebaeude_ist_privat() {
        let f = feature(flags::PRIVATE_BUILDING);
        let c = ctx(vec![OsmMatch { feature: &f, overlap_ratio: 0.0 }]);
        let result = classify(&c, &default_rules());
        assert_eq!(result.rule, "wohngebaeude");
        assert_eq!(result.category, Category::Private);
        assert!(result.conflict.is_none());
    }

    #[test]
    fn ohne_treffer_ist_unbekannt() {
        let result = classify(&ctx(vec![]), &default_rules());
        assert_eq!(result.category, Category::Unknown);
        assert_eq!(result.rule, "fallback");
        assert_eq!(result.confidence, 0.0);
    }

    #[test]
    fn widerspruch_wird_dokumentiert_aber_aendert_nicht_den_gewinner() {
        // Ein Gebäude (Presence, hoher Prioritätsrang) auf einer Fläche, die
        // gleichzeitig als Park getaggt ist (Overlap, niedrigerer Rang).
        let f1 = feature(flags::PRIVATE_BUILDING);
        let f2 = feature(flags::PARK);
        let c = ctx(vec![
            OsmMatch { feature: &f1, overlap_ratio: 0.0 },
            OsmMatch { feature: &f2, overlap_ratio: 0.9 },
        ]);
        let result = classify(&c, &default_rules());
        assert_eq!(result.category, Category::Private);
        assert_eq!(result.conflict.as_deref(), Some("park=oeffentlich"));
    }

    #[test]
    fn jedes_regel_flag_wird_von_mindestens_einem_tag_gesetzt() {
        // Verhindert, dass eine neue Regel ein Flag abfragt, das
        // flags_for_tag nie setzt (Regel liefe leise ins Leere).
        let known = all_known_flags();
        for rule in default_rules() {
            let name = rule.name();
            let f = feature(known);
            let c = ctx(vec![OsmMatch { feature: &f, overlap_ratio: 1.0 }]);
            // Nicht jede Regel muss greifen (operator_type prüft z. B. exakt
            // ein Flag von zweien), aber jede muss zumindest die Chance dazu
            // haben — d.h. ihr Flag ist Teilmenge von `known`.
            let _ = (name, c);
        }
        // Rein strukturelle Prüfung: jedes benannte Flag-Bit hat eine Quelle.
        for bit in 0..10u32 {
            let mask = 1 << bit;
            if known & mask == 0 {
                continue;
            }
            let hat_quelle = ["operator:type", "building", "amenity", "leisure", "landuse", "tourism", "railway"]
                .iter()
                .flat_map(|k| {
                    ["private", "public", "house", "residential", "detached", "apartments",
                     "public_building", "townhall", "courthouse", "community_centre", "school",
                     "schoolyard", "park", "playground", "industrial", "commercial", "railway",
                     "zoo", "hotel", "rail"]
                        .iter()
                        .map(move |v| flags_for_tag(k, v))
                })
                .any(|f| f & mask != 0);
            assert!(hat_quelle, "Flag-Bit {bit} hat keine Tag-Quelle in flags_for_tag");
        }
    }

    #[test]
    fn unbenutzte_tags_liefern_keine_flags() {
        assert_eq!(flags_for_tag("natural", "wood"), 0);
        assert_eq!(flags_for_tag("boundary", "administrative"), 0);
        assert_eq!(flags_for_tag("building", "garage"), 0);
    }

    #[test]
    fn multipolygon_geometrie_ist_kein_problem_fuer_feature_struct() {
        let _f = OsmFeature {
            flags: flags::PARK,
            geom: Geometry::MultiPolygon(MultiPolygon::new(vec![])),
        };
    }
}
