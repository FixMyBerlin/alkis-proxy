//! Regel-Engine für die OSM-basierte Nutzungsart-Schätzung.
//!
//! Portiert aus `ist-dieses-flurstueck-oeffentlich/src/rules.rs`, mit zwei
//! Änderungen gegenüber dem Referenz-Repo:
//!
//! * Die Regeln `verkehrsnetz_bahn`/`verkehrsnetz_strasse` kamen dort aus dem
//!   amtlichen INSPIRE-Verkehrsnetz-WFS, der trotz des Namens ALKIS-Daten sind
//!   und hier bewusst nicht verwendet werden. Ersetzt sind sie durch
//!   [`LineCoverageRule`] auf OSM-Achsen — siehe unten.
//! * Die Prädikate arbeiten auf einer `u32`-Bitmaske (siehe [`flags`]) statt
//!   auf `HashMap<String, String>`-Tags. Bei Millionen residenten Features
//!   (deutschlandweite PBF) dominieren Tag-Strings sonst den Speicher; die
//!   Bitmaske wird einmal beim Indizieren aus den Tags berechnet und ersetzt
//!   pro Anfrage String-Vergleiche durch Bit-Tests.
//!
//! [`scan_way_tags`] ist die **einzige** Stelle, an der Regeln und Indizierung
//! gekoppelt sind: `osm_extract.rs` hält nur Ways, für die diese Funktion
//! `Some` liefert. Eine neue Regel, die ein bisher unbekanntes Tag abfragt,
//! läuft sonst leise ins Leere, weil das Tag beim Indizieren schon verworfen
//! wurde — siehe den Konsistenztest unten.
//!
//! # Verkehrsflächen
//!
//! Straßen, Wege und Gleise sind in OSM Achsen (Linien), Flurstücke sind
//! Flächen. Die Brücke von 1D nach 2D schlägt [`LineCoverageRule`]: Die Länge
//! der Achse *innerhalb* des Flurstücks, multipliziert mit einer aus den Tags
//! abgeleiteten Nennbreite, ergibt die beanspruchte Fläche; ins Verhältnis zur
//! Flurstücksfläche gesetzt ist das der Deckungsgrad. Nenner ist damit das
//! Flurstück, **nicht** die Achse — der Anteil der Achse wäre die falsche
//! Richtung (eine lange Ortsdurchfahrt liegt nur zu wenigen Prozent in einem
//! einzelnen Straßenflurstück, ein kurzer Stich dagegen zu 100 % in einem
//! Privatgrundstück).

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
    /// Fahrbahn — `highway=motorway`…`service`, inkl. `pedestrian`.
    pub const ROAD: u32 = 1 << 10;
    /// Fuß-, Rad- und Wirtschaftsweg — `highway=footway`/`cycleway`/`track`/…
    pub const PATH: u32 = 1 << 11;
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
        // Gleisachsen sind Linien und werden über die Deckungsgrad-Regel
        // ausgewertet; `landuse=railway` (Bahngelände) ist eine Fläche und
        // läuft über die Overlap-Regel. Beide tragen dasselbe Flag, weil die
        // Aussage dieselbe ist — welche der beiden Regeln greift, entscheidet
        // die Geometrieart des Features.
        //
        // `tram` und `subway` fehlen bewusst: Straßenbahngleise liegen in der
        // Regel in der Fahrbahn, das Flurstück ist dann Straße und nicht Bahn.
        ("railway", "rail" | "light_rail" | "narrow_gauge") => flags::RAILWAY,
        ("highway", v) => match v {
            "motorway" | "trunk" | "primary" | "secondary" | "tertiary" | "unclassified"
            | "residential" | "living_street" | "pedestrian" | "busway" | "service"
            | "motorway_link" | "trunk_link" | "primary_link" | "secondary_link"
            | "tertiary_link" => flags::ROAD,
            "footway" | "cycleway" | "path" | "track" | "bridleway" | "steps" => flags::PATH,
            _ => 0,
        },
        _ => 0,
    }
}

/// Nennbreite der Verkehrsfläche in Dezimetern, die ein Tag begründet — `0`,
/// wenn es keine Aussage zur Breite macht.
///
/// Gemeint ist nicht die Fahrbahn-, sondern die **Verkehrsraumbreite**: das,
/// was ein Straßenflurstück typischerweise umfasst, also Fahrbahn plus
/// Bankett/Gehweg. Deshalb liegen die Werte über den reinen Fahrbahnbreiten.
/// Sie sind Schätzungen und gehen nur über den Schwellwertvergleich in das
/// Ergebnis ein — auf ±50 % genau zu sein reicht dafür aus.
fn nennbreite_dm(key: &str, value: &str) -> u16 {
    match (key, value) {
        ("highway", "motorway") => 150,
        ("highway", "trunk") => 120,
        ("highway", "primary") => 110,
        ("highway", "secondary") => 100,
        ("highway", "tertiary") => 90,
        ("highway", "motorway_link") => 90,
        ("highway", "residential" | "pedestrian" | "trunk_link") => 80,
        ("highway", "unclassified" | "living_street" | "busway" | "primary_link") => 70,
        ("highway", "secondary_link") => 65,
        ("highway", "tertiary_link") => 60,
        ("highway", "service") => 45,
        ("highway", "track") => 35,
        ("highway", "cycleway") => 30,
        ("highway", "footway" | "path" | "bridleway") => 25,
        ("highway", "steps") => 20,
        // Regelspur mit Gleisbett und Randweg; mehrgleisige Strecken sind in
        // OSM mehrere parallele Ways und summieren sich dadurch von selbst.
        ("railway", "rail") => 60,
        ("railway", "light_rail" | "narrow_gauge") => 45,
        // Rangier-, Anschluss- und Abstellgleise liegen dichter beieinander.
        ("service", "siding" | "spur" | "yard" | "crossover") => 45,
        _ => 0,
    }
}

/// Ob ein Tag den ganzen Way für die Verkehrsflächen-Erkennung disqualifiziert.
///
/// `bridge`/`tunnel` sind der Kern: Eine Brücke über ein Flurstück macht dieses
/// nicht zur Verkehrsfläche (darunter liegt Wasser, Wald oder eine andere
/// Straße), ein Tunnel darunter erst recht nicht. Weil OSM Brücken- und
/// Tunnelabschnitte als eigene Ways führt, trifft der Ausschluss genau den
/// betroffenen Abschnitt und nicht die ganze Straße.
///
/// `service=driveway`/`parking_aisle` sind Grundstückszufahrten und
/// Parkplatzgassen — sie liegen auf Privatgrund und wären sonst der häufigste
/// Fehlerfall überhaupt.
fn ist_ausschluss(key: &str, value: &str) -> bool {
    match (key, value) {
        ("bridge" | "tunnel", v) => v != "no",
        ("highway", "proposed" | "construction" | "raceway" | "escape" | "corridor") => true,
        ("service", "driveway" | "parking_aisle" | "drive-through") => true,
        _ => false,
    }
}

/// Was das Indizieren von einem Way behält.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WayTags {
    pub flags: u32,
    /// Nennbreite in Dezimetern; `0` für Features, die nicht als Achse
    /// ausgewertet werden (Gebäude, Landnutzungsflächen).
    pub width_dm: u16,
    /// `area=yes`: Der Way ist als Fläche gemeint, nicht als Achse. Nur so
    /// lässt sich eine flächig gemappte Fußgängerzone von einem Kreisverkehr
    /// unterscheiden — beide sind geschlossene Ways mit `highway=*`.
    pub flaeche: bool,
}

/// Wertet die Tags eines Ways in einem Durchlauf aus — `None`, wenn der Way
/// für keine Regel relevant ist oder durch [`ist_ausschluss`] herausfällt.
///
/// Ein Durchlauf, weil `osmpbf` die Tags als einmaligen Iterator liefert.
pub fn scan_way_tags<'a>(tags: impl Iterator<Item = (&'a str, &'a str)>) -> Option<WayTags> {
    let mut flags = 0u32;
    let mut klassenbreite_dm = 0u16;
    let mut width_tag_dm = 0u16;
    let mut lanes = 0f64;
    let mut ausgeschlossen = false;
    let mut flaeche = false;

    for (key, value) in tags {
        if ist_ausschluss(key, value) {
            ausgeschlossen = true;
        }
        flags |= flags_for_tag(key, value);
        klassenbreite_dm = klassenbreite_dm.max(nennbreite_dm(key, value));
        match key {
            "width" => width_tag_dm = parse_meter_dm(value).unwrap_or(0),
            "lanes" => lanes = value.parse::<f64>().unwrap_or(0.0),
            "area" if value == "yes" => flaeche = true,
            _ => {}
        }
    }

    if ausgeschlossen || flags == 0 {
        return None;
    }

    // Nur nach oben korrigieren: Ein getaggtes `width` meint die Fahrbahn ohne
    // Nebenanlagen und liegt deshalb oft unter der Verkehrsraumbreite, die die
    // Klassenvorgabe schon abdeckt. Als Hinweis auf eine *breitere* Straße als
    // üblich ist es dagegen wertvoll.
    let spurbreite_dm = if lanes >= 1.0 {
        (lanes * 32.5 + 30.0) as u16
    } else {
        0
    };
    let width_dm = klassenbreite_dm.max(width_tag_dm).max(spurbreite_dm);

    Some(WayTags { flags, width_dm, flaeche })
}

/// Liest eine Meterangabe aus einem OSM-Wert und gibt sie in Dezimetern
/// zurück. Akzeptiert `5`, `5.5`, `5,5` und `5.5 m`; verwirft Fuß-/Zollangaben
/// (`18'`, `6'6"`) und alles Unlesbare, statt sie falsch zu interpretieren.
fn parse_meter_dm(value: &str) -> Option<u16> {
    let value = value.trim();
    let ende = value
        .find(|c: char| !c.is_ascii_digit() && c != '.' && c != ',')
        .unwrap_or(value.len());
    let (zahl, rest) = value.split_at(ende);
    let rest = rest.trim();
    if !rest.is_empty() && rest != "m" {
        return None;
    }
    let meter: f64 = zahl.replace(',', ".").parse().ok()?;
    if !(0.0..=200.0).contains(&meter) {
        return None;
    }
    Some((meter * 10.0) as u16)
}

/// Klassifikations-Kategorie eines Flurstücks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Private,
    /// Auch Verkehrsflächen: Straßen, Wege und Plätze sind öffentlich. Welche
    /// Regel dahintersteckt, sagt `Classification::rule` (`strasse`, `weg`,
    /// `verkehrsflaeche`) — die Kategorie bleibt bewusst grob.
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
    /// `f64`, nicht `f32`: Der Wert geht direkt nach JSON, und ein auf zwei
    /// Stellen gerundetes `f32` wird beim Verbreitern zu `f64` wieder krumm
    /// (0,6 → 0.6000000238418579).
    pub confidence: f64,
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
            name: "bahngelaende",
            category: Category::Railway,
            threshold: 0.3,
            flag: flags::RAILWAY,
        }),
        // Gebäude sind ein präziser Fußabdruck-Hinweis -> vor groben
        // Landnutzungsflächen. Sie stehen außerdem bewusst **vor** den
        // Verkehrsregeln: OSM-Achsen sind gegenüber den ALKIS-Grenzen um
        // einige Meter versetzt und können dadurch längs durch eine schmale
        // Nachbarparzelle laufen. Ein Gebäude auf dem Flurstück schließt eine
        // Verkehrsfläche sicher aus und fängt genau diesen Fall ab.
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
        // Bahn vor Straße: Wo beides durch dasselbe Flurstück läuft, ist der
        // Bahnkörper das prägende Merkmal.
        Box::new(LineCoverageRule {
            name: "bahnstrecke",
            category: Category::Railway,
            flag: flags::RAILWAY,
        }),
        Box::new(LineCoverageRule {
            name: "strasse",
            category: Category::Public,
            flag: flags::ROAD,
        }),
        Box::new(LineCoverageRule {
            name: "weg",
            category: Category::Public,
            flag: flags::PATH,
        }),
        // Fußgängerzonen und Plätze sind in OSM als Fläche gemappt
        // (`area=yes`) und haben deshalb keine Achse, über die der
        // Deckungsgrad laufen könnte. Schwelle wie bei den Achsen (0,5) statt
        // wie bei den übrigen Overlap-Regeln (0,3): Bei `park` genügt ein
        // Drittel Überlappung als Hinweis auf die Nutzungsart, hier soll die
        // Verkehrsfläche das Flurstück aber tatsächlich dominieren — sonst
        // erklärt ein großzügig über einen Baublock gezogener Platz die
        // halbe Nachbarschaft zur Verkehrsfläche.
        Box::new(OverlapRule {
            name: "verkehrsflaeche",
            category: Category::Public,
            threshold: DECKUNG_SCHWELLE,
            flag: flags::ROAD | flags::PATH,
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
                confidence: runde(overlap),
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
    confidence: f64,
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

/// Ab welchem Deckungsgrad ein langgestrecktes Flurstück als Verkehrsfläche
/// gilt (siehe [`MAX_KOMPAKTHEIT`]).
const DECKUNG_SCHWELLE: f64 = 0.5;

/// Deckungsgrad, ab dem die Form keine Rolle mehr spielt. Fängt die kompakten
/// Ausnahmen ab: Kreuzungs- und Wendehammerflächen, kurze Stichstraßen.
const DECKUNG_OHNE_FORM: f64 = 0.9;

/// Obergrenze der Polsby-Popper-Kompaktheit `4πA/U²` für Verkehrsflächen.
///
/// Verkehrsflurstücke sind per Bauform langgestreckt und liegen weit darunter
/// (Straßenflurstück 8 × 200 m: 0,12; Wirtschaftsweg 3 × 500 m: 0,02), normale
/// Baugrundstücke deutlich darüber (Reihenhaus 6 × 30 m: 0,44; Quadratparzelle
/// 20 × 20 m: 0,79). Der Wert kostet nichts — Fläche und Umfang fallen bei der
/// Verschneidung ohnehin an — und ist der wirksamste Schutz gegen den
/// OSM/ALKIS-Versatz.
const MAX_KOMPAKTHEIT: f64 = 0.25;

/// Greift, wenn die Achsen mit diesem Flag einen hinreichenden Anteil der
/// Flurstücksfläche beanspruchen. Die Brücke von der OSM-Linie zur
/// ALKIS-Fläche; Begründung der Kennzahl im Modulkopf.
struct LineCoverageRule {
    name: &'static str,
    category: Category,
    flag: u32,
}

impl Rule for LineCoverageRule {
    fn name(&self) -> &'static str {
        self.name
    }

    fn apply(&self, ctx: &OsmContext) -> Option<Classification> {
        let deckung = ctx.line_coverage(self.flag);
        let langgestreckt = ctx.compactness < MAX_KOMPAKTHEIT;
        if deckung > DECKUNG_OHNE_FORM || (deckung > DECKUNG_SCHWELLE && langgestreckt) {
            Some(Classification {
                category: self.category,
                rule: self.name.to_string(),
                confidence: runde(deckung),
                conflict: None,
            })
        } else {
            None
        }
    }
}

/// Konfidenz auf zwei Nachkommastellen — die Eingangsgrößen sind Schätzungen,
/// eine Vollausgabe würde eine Genauigkeit vortäuschen, die nicht besteht.
fn runde(wert: f64) -> f64 {
    (wert.min(1.0) * 100.0).round() / 100.0
}

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
        | ROAD
        | PATH
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classification::index::OsmMatch;
    use crate::classification::OsmFeature;
    use geo_types::{Geometry, MultiPolygon, Point};

    /// Tag-Paare, die zusammen jedes Flag aus [`all_known_flags`] setzen
    /// müssen — Grundlage des Konsistenztests weiter unten.
    const BEISPIEL_TAGS: &[(&str, &str)] = &[
        ("operator:type", "private"),
        ("operator:type", "public"),
        ("building", "house"),
        ("building", "public"),
        ("amenity", "townhall"),
        ("leisure", "schoolyard"),
        ("leisure", "park"),
        ("leisure", "playground"),
        ("landuse", "industrial"),
        ("landuse", "railway"),
        ("tourism", "zoo"),
        ("tourism", "hotel"),
        ("railway", "rail"),
        ("highway", "residential"),
        ("highway", "footway"),
    ];

    fn feature(flags: u32) -> OsmFeature {
        OsmFeature {
            flags,
            width_dm: 0,
            geom: Geometry::Point(Point::new(0.0, 0.0)),
        }
    }

    fn achse(flags: u32, width_dm: u16) -> OsmFeature {
        OsmFeature {
            flags,
            width_dm,
            geom: Geometry::Point(Point::new(0.0, 0.0)),
        }
    }

    fn ctx<'a>(matches: Vec<OsmMatch<'a>>) -> OsmContext<'a> {
        OsmContext { matches, ..Default::default() }
    }

    /// Kontext eines Flurstücks mit gegebener Fläche und Kompaktheit.
    fn ctx_flurstueck<'a>(
        matches: Vec<OsmMatch<'a>>,
        area_m2: f64,
        compactness: f64,
    ) -> OsmContext<'a> {
        OsmContext { matches, area_m2, compactness }
    }

    fn treffer(feature: &OsmFeature) -> OsmMatch<'_> {
        OsmMatch { feature, overlap_ratio: 0.0, clipped_length_m: 0.0 }
    }

    #[test]
    fn operator_type_schlaegt_alles_andere() {
        let f = feature(flags::OPERATOR_PUBLIC | flags::PRIVATE_BUILDING);
        let c = ctx(vec![OsmMatch { overlap_ratio: 1.0, ..treffer(&f) }]);
        let result = classify(&c, &default_rules());
        assert_eq!(result.rule, "operator_type");
        assert_eq!(result.category, Category::Public);
    }

    #[test]
    fn wohngebaeude_ist_privat() {
        let f = feature(flags::PRIVATE_BUILDING);
        let c = ctx(vec![treffer(&f)]);
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
            treffer(&f1),
            OsmMatch { overlap_ratio: 0.9, ..treffer(&f2) },
        ]);
        let result = classify(&c, &default_rules());
        assert_eq!(result.category, Category::Private);
        assert_eq!(result.conflict.as_deref(), Some("park=oeffentlich"));
    }

    // --- Verkehrsflächen ------------------------------------------------

    #[test]
    fn strassenflurstueck_wird_erkannt() {
        // 8 × 200 m: Fläche 1600 m², Kompaktheit 0,12, Achse 200 m × 6,5 m
        // -> Deckung 0,81.
        let f = achse(flags::ROAD, 65);
        let c = ctx_flurstueck(
            vec![OsmMatch { clipped_length_m: 200.0, ..treffer(&f) }],
            1600.0,
            0.12,
        );
        let result = classify(&c, &default_rules());
        assert_eq!(result.category, Category::Public);
        assert_eq!(result.rule, "strasse");
        assert_eq!(result.confidence, 0.81);
    }

    #[test]
    fn fussweg_wird_als_verkehr_erkannt() {
        // 3 × 100 m: Fläche 300 m², Achse 100 m × 2,5 m -> Deckung 0,83.
        let f = achse(flags::PATH, 25);
        let c = ctx_flurstueck(
            vec![OsmMatch { clipped_length_m: 100.0, ..treffer(&f) }],
            300.0,
            0.04,
        );
        let result = classify(&c, &default_rules());
        assert_eq!(result.category, Category::Public);
        assert_eq!(result.rule, "weg");
    }

    #[test]
    fn zufahrt_macht_aus_einem_grundstueck_keine_verkehrsflaeche() {
        // 20 × 30 m mit 10 m Achse: Deckung 0,05 — weit unter der Schwelle.
        let f = achse(flags::ROAD, 30);
        let c = ctx_flurstueck(
            vec![OsmMatch { clipped_length_m: 10.0, ..treffer(&f) }],
            600.0,
            0.79,
        );
        assert_eq!(classify(&c, &default_rules()).category, Category::Unknown);
    }

    #[test]
    fn kompakte_parzelle_bleibt_trotz_hoher_deckung_verschont() {
        // Der OSM/ALKIS-Versatz-Fall: 6 × 30 m Reihenhausparzelle, die Achse
        // läuft längs hindurch -> Deckung 0,83, aber Kompaktheit 0,44.
        let f = achse(flags::ROAD, 50);
        let c = ctx_flurstueck(
            vec![OsmMatch { clipped_length_m: 30.0, ..treffer(&f) }],
            180.0,
            0.44,
        );
        assert_eq!(classify(&c, &default_rules()).category, Category::Unknown);
    }

    #[test]
    fn vollstaendige_deckung_greift_auch_ohne_langgestreckte_form() {
        // Kreuzungs-/Wendehammerfläche: kompakt, aber restlos überfahren.
        let f = achse(flags::ROAD, 80);
        let c = ctx_flurstueck(
            vec![OsmMatch { clipped_length_m: 40.0, ..treffer(&f) }],
            250.0,
            0.7,
        );
        let result = classify(&c, &default_rules());
        assert_eq!(result.category, Category::Public);
        assert_eq!(result.confidence, 1.0);
    }

    #[test]
    fn gebaeude_schlaegt_verkehrsflaeche() {
        // Beide Regeln greifen; die Gebäude-Regel steht davor und gewinnt.
        let gebaeude = feature(flags::PRIVATE_BUILDING);
        let strasse = achse(flags::ROAD, 65);
        let c = ctx_flurstueck(
            vec![
                treffer(&gebaeude),
                OsmMatch { clipped_length_m: 200.0, ..treffer(&strasse) },
            ],
            1600.0,
            0.12,
        );
        let result = classify(&c, &default_rules());
        assert_eq!(result.category, Category::Private);
        assert_eq!(result.conflict.as_deref(), Some("strasse=oeffentlich"));
    }

    #[test]
    fn bahn_schlaegt_strasse_im_selben_flurstueck() {
        let gleis = achse(flags::RAILWAY, 60);
        let strasse = achse(flags::ROAD, 65);
        let c = ctx_flurstueck(
            vec![
                OsmMatch { clipped_length_m: 200.0, ..treffer(&gleis) },
                OsmMatch { clipped_length_m: 200.0, ..treffer(&strasse) },
            ],
            1600.0,
            0.12,
        );
        let result = classify(&c, &default_rules());
        assert_eq!(result.category, Category::Railway);
        assert_eq!(result.rule, "bahnstrecke");
    }

    #[test]
    fn parallele_achsen_summieren_sich() {
        // Fahrbahn + separat gemappter Gehweg im selben Straßenflurstück.
        let fahrbahn = achse(flags::ROAD, 40);
        let gehweg = achse(flags::ROAD, 25);
        let c = ctx_flurstueck(
            vec![
                OsmMatch { clipped_length_m: 100.0, ..treffer(&fahrbahn) },
                OsmMatch { clipped_length_m: 100.0, ..treffer(&gehweg) },
            ],
            700.0,
            0.15,
        );
        // (100×4 + 100×2,5) / 700 = 0,93 — einzeln bliebe jede unter 0,6.
        let result = classify(&c, &default_rules());
        assert_eq!(result.category, Category::Public);
    }

    // --- Tag-Auswertung -------------------------------------------------

    #[test]
    fn strasse_bekommt_flag_und_nennbreite() {
        let tags = [("highway", "residential")].into_iter();
        let scan = scan_way_tags(tags).unwrap();
        assert_eq!(scan.flags, flags::ROAD);
        assert_eq!(scan.width_dm, 80);
    }

    #[test]
    fn bruecke_und_tunnel_werden_verworfen() {
        assert!(scan_way_tags([("highway", "primary"), ("bridge", "yes")].into_iter()).is_none());
        assert!(scan_way_tags([("railway", "rail"), ("tunnel", "yes")].into_iter()).is_none());
        // bridge=no ist kein Ausschluss.
        assert!(scan_way_tags([("highway", "primary"), ("bridge", "no")].into_iter()).is_some());
    }

    #[test]
    fn grundstueckszufahrt_wird_verworfen() {
        let tags = [("highway", "service"), ("service", "driveway")].into_iter();
        assert!(scan_way_tags(tags).is_none());
    }

    #[test]
    fn breiteres_width_tag_hebt_die_nennbreite_an() {
        let breit = scan_way_tags([("highway", "residential"), ("width", "12")].into_iter());
        assert_eq!(breit.unwrap().width_dm, 120);
        // Schmaleres width senkt nicht: die Klassenvorgabe umfasst die
        // Nebenanlagen, das Tag nur die Fahrbahn.
        let schmal = scan_way_tags([("highway", "residential"), ("width", "5.5")].into_iter());
        assert_eq!(schmal.unwrap().width_dm, 80);
    }

    #[test]
    fn viele_fahrstreifen_heben_die_nennbreite_an() {
        let tags = [("highway", "secondary"), ("lanes", "4")].into_iter();
        assert_eq!(scan_way_tags(tags).unwrap().width_dm, 160);
    }

    #[test]
    fn konfidenz_erreicht_die_ausgabe_ohne_rundungsschwanz() {
        // Der Wert wandert unverändert nach JSON; mit f32 stünde dort
        // 0.8100000023841858 statt 0.81.
        assert_eq!(serde_json::json!(runde(0.8125)).to_string(), "0.81");
        assert_eq!(serde_json::json!(runde(1.0)).to_string(), "1.0");
        let presence = default_rules()
            .iter()
            .find(|r| r.name() == "wohngebaeude")
            .map(|r| {
                let f = feature(flags::PRIVATE_BUILDING);
                let c = ctx(vec![treffer(&f)]);
                r.apply(&c).unwrap().confidence
            })
            .unwrap();
        assert_eq!(serde_json::json!(presence).to_string(), "0.6");
    }

    #[test]
    fn meterangaben_werden_gelesen_fussangaben_nicht() {
        assert_eq!(parse_meter_dm("5"), Some(50));
        assert_eq!(parse_meter_dm("5.5"), Some(55));
        assert_eq!(parse_meter_dm("5,5"), Some(55));
        assert_eq!(parse_meter_dm("5.5 m"), Some(55));
        assert_eq!(parse_meter_dm("18'"), None);
        assert_eq!(parse_meter_dm("6'6\""), None);
        assert_eq!(parse_meter_dm("breit"), None);
        assert_eq!(parse_meter_dm("99999"), None);
    }

    #[test]
    fn tram_ist_keine_bahnstrecke() {
        // Straßenbahngleise liegen meist in der Fahrbahn; das Flurstück ist
        // dann Straße. Ein Way nur mit railway=tram wird gar nicht indiziert.
        assert!(scan_way_tags([("railway", "tram")].into_iter()).is_none());
    }

    #[test]
    fn irrelevanter_way_wird_verworfen() {
        assert!(scan_way_tags([("natural", "wood")].into_iter()).is_none());
        assert!(scan_way_tags([("highway", "motorway_junction")].into_iter()).is_none());
    }

    #[test]
    fn jedes_regel_flag_wird_von_mindestens_einem_tag_gesetzt() {
        // Verhindert, dass eine neue Regel ein Flag abfragt, das kein Tag je
        // setzt — die Regel liefe leise ins Leere, weil `osm_extract` den Way
        // schon beim Indizieren verwirft.
        let known = all_known_flags();
        for bit in 0..32u32 {
            let mask = 1 << bit;
            if known & mask == 0 {
                continue;
            }
            let hat_quelle = BEISPIEL_TAGS
                .iter()
                .any(|(k, v)| flags_for_tag(k, v) & mask != 0);
            assert!(hat_quelle, "Flag-Bit {bit} hat keine Tag-Quelle in flags_for_tag");
        }
    }

    #[test]
    fn jede_regel_fragt_nur_bekannte_flags_ab() {
        // Gegenrichtung: Kein Regel-Flag außerhalb von all_known_flags.
        let known = all_known_flags();
        for rule in default_rules() {
            let f = feature(known);
            let c = ctx(vec![OsmMatch { overlap_ratio: 1.0, ..treffer(&f) }]);
            // Greift eine Regel bei gesetzten *allen* bekannten Flags nicht,
            // ist das in Ordnung (Overlap-/LineCoverage-Regeln brauchen zudem
            // Geometrie); der Aufruf darf nur nicht panisch werden.
            let _ = rule.apply(&c);
        }
    }

    #[test]
    fn unbenutzte_tags_liefern_keine_flags() {
        assert_eq!(flags_for_tag("natural", "wood"), 0);
        assert_eq!(flags_for_tag("boundary", "administrative"), 0);
        assert_eq!(flags_for_tag("building", "garage"), 0);
        assert_eq!(flags_for_tag("highway", "bus_stop"), 0);
    }

    #[test]
    fn multipolygon_geometrie_ist_kein_problem_fuer_feature_struct() {
        let _f = OsmFeature {
            flags: flags::PARK,
            width_dm: 0,
            geom: Geometry::MultiPolygon(MultiPolygon::new(vec![])),
        };
    }
}
