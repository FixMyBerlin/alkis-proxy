//! Flurstückskennzeichen: Parsen, Normalisieren, Zerlegen.
//!
//! Das amtliche Flurstückskennzeichen der AdV folgt der Struktur
//!
//! ```text
//! Land(2) · Gemarkung(4) · Flur(3) · Zähler(5) · Nenner(4) · Folge(2)   = 20 Zeichen
//! ```
//!
//! Die Bundesländer liefern es aber in unvereinbaren Ausprägungen. Real beobachtet:
//!
//! | Land | Rohwert                | Länge | Leerstellen |
//! |------|------------------------|-------|-------------|
//! | BE   | `11000172000506____`   | 18    | `_`         |
//! | BW   | `08146000000094000100` | 20    | keine       |
//! | SH   | `01253801600432______` | 20    | `_`         |
//!
//! Zusätzlich kommen Schreibweisen mit Trennzeichen vor (`052632-001-00123/0000`).
//!
//! Dieses Modul führt alle Varianten auf eine kanonische 20-Zeichen-Form zurück und
//! macht die Komponenten einzeln zugänglich. Das ist doppelt nützlich: es erlaubt den
//! Abgleich von Kennzeichen über Landesgrenzen hinweg, und es rekonstruiert für
//! INSPIRE-Quellen (die nur das Kennzeichen liefern) die fehlenden Einzelfelder
//! Gemarkung, Flur, Zähler und Nenner.

use std::fmt;

/// Zeichen, das eine nicht belegte Stelle im Kennzeichen markiert.
const FILL: char = '_';

/// Feldbreiten in Zeichen, in der Reihenfolge des Kennzeichens.
const W_LAND: usize = 2;
const W_GEMARKUNG: usize = 4;
const W_FLUR: usize = 3;
const W_ZAEHLER: usize = 5;
const W_NENNER: usize = 4;
const W_FOLGE: usize = 2;

/// Kürzeste akzeptierte Form: bis einschließlich Zähler. Alles danach (Nenner,
/// Folge) darf fehlen und gilt dann als unbelegt.
const LEN_MIN: usize = W_LAND + W_GEMARKUNG + W_FLUR + W_ZAEHLER;
/// Kanonische Gesamtlänge.
pub const LEN_CANONICAL: usize =
    LEN_MIN + W_NENNER + W_FOLGE;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ParcelIdError {
    #[error("Kennzeichen ist leer")]
    Empty,
    #[error("Kennzeichen hat {got} Zeichen, erwartet {LEN_MIN}..={LEN_CANONICAL}: {raw:?}")]
    BadLength { raw: String, got: usize },
    #[error("Kennzeichen enthält unerwartetes Zeichen {ch:?}: {raw:?}")]
    BadChar { raw: String, ch: char },
    #[error("Pflichtfeld {field} ist nicht belegt: {raw:?}")]
    MissingField { raw: String, field: &'static str },
}

/// Ein zerlegtes Flurstückskennzeichen.
///
/// Land, Gemarkung, Flur und Zähler sind immer belegt; Nenner und Folge können fehlen
/// (im Rohwert als Füllzeichen dargestellt).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ParcelId {
    land: String,
    gemarkung: String,
    flur: Option<String>,
    zaehler: String,
    nenner: Option<String>,
    folge: Option<String>,
}

impl ParcelId {
    /// Zerlegt einen Rohwert beliebiger Landesausprägung.
    ///
    /// Toleriert Trennzeichen (`-`, `/`, `.`), Leerzeichen als Füllzeichen und eine
    /// fehlende Folgenummer.
    pub fn parse(raw: &str) -> Result<Self, ParcelIdError> {
        let raw = raw.trim();
        if raw.is_empty() {
            return Err(ParcelIdError::Empty);
        }

        // Trennzeichen entfernen, Leerzeichen als Füllzeichen vereinheitlichen.
        let mut cleaned = String::with_capacity(LEN_CANONICAL);
        for ch in raw.chars() {
            match ch {
                '-' | '/' | '.' => continue,
                ' ' => cleaned.push(FILL),
                // Buchstaben kommen vor: Zähler und Nenner tragen mitunter
                // alphanumerische Zusätze (Sachsen liefert etwa
                // "140208___00952000a02" — Nenner "000a").
                c if c.is_ascii_alphanumeric() || c == FILL => cleaned.push(c),
                c => {
                    return Err(ParcelIdError::BadChar {
                        raw: raw.to_string(),
                        ch: c,
                    })
                }
            }
        }

        // Rechts fehlende Stellen gelten als unbelegt. Das deckt sowohl Länder ab,
        // die die Folgenummer weglassen (Berlin, 18 Zeichen), als auch Werte, deren
        // Leerzeichen-Padding beim Trimmen verloren ging.
        if !(LEN_MIN..=LEN_CANONICAL).contains(&cleaned.len()) {
            return Err(ParcelIdError::BadLength {
                got: cleaned.len(),
                raw: raw.to_string(),
            });
        }
        while cleaned.len() < LEN_CANONICAL {
            cleaned.push(FILL);
        }

        let mut cut = Cutter::new(&cleaned);
        let land = cut.take(W_LAND);
        let gemarkung = cut.take(W_GEMARKUNG);
        let flur = cut.take(W_FLUR);
        let zaehler = cut.take(W_ZAEHLER);
        let nenner = cut.take(W_NENNER);
        let folge = cut.take(W_FOLGE);

        let required = |field: &'static str, value: &str| -> Result<String, ParcelIdError> {
            if is_blank(value) {
                Err(ParcelIdError::MissingField {
                    raw: raw.to_string(),
                    field,
                })
            } else {
                Ok(value.to_string())
            }
        };

        Ok(ParcelId {
            land: required("Land", land)?,
            gemarkung: required("Gemarkung", gemarkung)?,
            // Nicht jedes Land führt Fluren: Sachsen lässt die Stelle leer
            // (`140208___00214000300`), Baden-Württemberg schreibt Nullen.
            flur: optional(flur),
            zaehler: required("Zähler", zaehler)?,
            nenner: optional(nenner),
            folge: optional(folge),
        })
    }

    /// Kanonische 20-Zeichen-Form; nicht belegte Stellen als `_`.
    ///
    /// Dadurch bilden `…0000…` (Baden-Württemberg) und `…____…` (übrige Länder)
    /// auf denselben Schlüssel ab — Voraussetzung dafür, dass die
    /// Deduplizierung über Kachelgrenzen zuverlässig greift.
    pub fn canonical(&self) -> String {
        let mut s = String::with_capacity(LEN_CANONICAL);
        s.push_str(&self.land);
        s.push_str(&self.gemarkung);
        s.push_str(&fill_or(self.flur.as_deref(), W_FLUR));
        s.push_str(&self.zaehler);
        s.push_str(&fill_or(self.nenner.as_deref(), W_NENNER));
        s.push_str(&fill_or(self.folge.as_deref(), W_FOLGE));
        s
    }

    /// Amtlicher Länderschlüssel, zweistellig (`"08"`).
    pub fn land(&self) -> &str {
        &self.land
    }

    /// Bundesland-Kürzel (`"BW"`), sofern der Länderschlüssel bekannt ist.
    pub fn bundesland(&self) -> Option<&'static str> {
        bundesland_from_land_key(&self.land)
    }

    /// Gemarkungsnummer innerhalb des Landes, vierstellig (`"1460"`).
    ///
    /// Für sich genommen **nicht** bundesweit eindeutig — dafür
    /// [`gemarkung_schluessel`](Self::gemarkung_schluessel) verwenden.
    pub fn gemarkung(&self) -> &str {
        &self.gemarkung
    }

    /// Bundesweit eindeutiger Gemarkungsschlüssel: Land + Gemarkung, sechsstellig
    /// (`"081460"`).
    ///
    /// Entspricht dem, was die AVE-Dienste als `gemaschl` liefern — dort aber nicht
    /// überall vorhanden (Thüringen lässt das Feld weg), weshalb der Wert hier aus
    /// dem Kennzeichen abgeleitet wird.
    pub fn gemarkung_schluessel(&self) -> String {
        format!("{}{}", self.land, self.gemarkung)
    }

    /// Flurnummer ohne führende Nullen (`"31"`), sofern das Land Fluren führt.
    ///
    /// Sachsen und Baden-Württemberg kennen keine Fluren; dort ist der Wert
    /// `None`.
    pub fn flur(&self) -> Option<String> {
        self.flur.as_deref().map(strip_leading_zeros)
    }

    /// Zähler ohne führende Nullen (`"94"`).
    pub fn zaehler(&self) -> String {
        strip_leading_zeros(&self.zaehler)
    }

    /// Nenner ohne führende Nullen (`"1"`), sofern belegt.
    pub fn nenner(&self) -> Option<String> {
        self.nenner.as_deref().map(strip_leading_zeros)
    }

    /// Folgenummer ohne führende Nullen, sofern belegt.
    pub fn folge(&self) -> Option<String> {
        self.folge.as_deref().map(strip_leading_zeros)
    }

    /// Sprechende Flurstücksnummer: `"94/1"` mit Nenner, sonst `"94"`.
    pub fn flurstuecksnummer(&self) -> String {
        match self.nenner() {
            Some(n) => format!("{}/{}", self.zaehler(), n),
            None => self.zaehler(),
        }
    }
}

impl fmt::Display for ParcelId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.canonical())
    }
}

/// Schneidet einen String in Felder fester Breite.
struct Cutter<'a> {
    rest: &'a str,
}

impl<'a> Cutter<'a> {
    fn new(s: &'a str) -> Self {
        Cutter { rest: s }
    }
    /// Nimmt die nächsten `n` Zeichen. Die Eingabe ist zu diesem Zeitpunkt
    /// garantiert ASCII (Ziffern und `_`), Byte-Slicing ist daher sicher.
    fn take(&mut self, n: usize) -> &'a str {
        let (head, tail) = self.rest.split_at(n);
        self.rest = tail;
        head
    }
}

/// Ein Pflichtfeld gilt als unbelegt, wenn es ausschließlich aus Füllzeichen besteht.
fn is_blank(s: &str) -> bool {
    s.chars().all(|c| c == FILL)
}

/// Optionale Felder (Nenner, Folge) gelten zusätzlich als unbelegt, wenn sie nur
/// aus Nullen bestehen.
///
/// Die Länder drücken „nicht vorhanden" unterschiedlich aus: Berlin, NRW und
/// Schleswig-Holstein schreiben Füllzeichen (`____`), Baden-Württemberg schreibt
/// Nullen (`0000`). Fachlich gibt es weder Nenner noch Folge mit dem Wert 0 —
/// ein Flurstück heißt `634`, nicht `634/0`. Beide Schreibweisen werden deshalb
/// gleich behandelt, sonst hinge die Flurstücksnummer vom Bundesland ab.
fn optional(s: &str) -> Option<String> {
    let unbelegt = s.chars().all(|c| c == FILL || c == '0');
    if unbelegt {
        None
    } else {
        Some(s.to_string())
    }
}

fn fill_or(value: Option<&str>, width: usize) -> String {
    match value {
        Some(v) => v.to_string(),
        None => FILL.to_string().repeat(width),
    }
}

/// Entfernt führende Nullen, lässt aber `"0"` stehen statt einen leeren String zu erzeugen.
fn strip_leading_zeros(s: &str) -> String {
    let trimmed = s.trim_start_matches('0');
    if trimmed.is_empty() {
        "0".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Amtliche Länderschlüssel nach AdV.
pub fn bundesland_from_land_key(key: &str) -> Option<&'static str> {
    Some(match key {
        "01" => "SH",
        "02" => "HH",
        "03" => "NI",
        "04" => "HB",
        "05" => "NW",
        "06" => "HE",
        "07" => "RP",
        "08" => "BW",
        "09" => "BY",
        "10" => "SL",
        "11" => "BE",
        "12" => "BB",
        "13" => "MV",
        "14" => "SN",
        "15" => "ST",
        "16" => "TH",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Die folgenden drei Rohwerte stammen aus echten GetFeature-Antworten der
    // jeweiligen Landesdienste und decken alle beobachteten Ausprägungen ab.

    #[test]
    fn berlin_18_stellig_ohne_folge() {
        let id = ParcelId::parse("11000172000506____").unwrap();
        assert_eq!(id.land(), "11");
        assert_eq!(id.bundesland(), Some("BE"));
        assert_eq!(id.gemarkung(), "0001"); // Berlin liefert gmk="0001"
        assert_eq!(id.flur().as_deref(), Some("720")); // fln="720"
        assert_eq!(id.zaehler(), "506"); // zae="506"
        assert_eq!(id.nenner(), None); // nen="" → unbelegt
        assert_eq!(id.folge(), None); // im Rohwert nicht vorhanden
        assert_eq!(id.flurstuecksnummer(), "506");
        // Kanonisch auf 20 Zeichen aufgefüllt.
        assert_eq!(id.canonical(), "11000172000506______");
    }

    #[test]
    fn baden_wuerttemberg_20_stellig_voll_belegt() {
        let id = ParcelId::parse("08146000000094000100").unwrap();
        assert_eq!(id.bundesland(), Some("BW"));
        assert_eq!(id.gemarkung(), "1460"); // gemarkung_id=1460
        assert_eq!(id.gemarkung_schluessel(), "081460");
        assert_eq!(id.flur(), None); // BW führt keine Fluren
        assert_eq!(id.zaehler(), "94"); // zaehler=94
        assert_eq!(id.nenner(), Some("1".into())); // nenner=1
        assert_eq!(id.folge(), None); // folgenummer=0 → unbelegt
        assert_eq!(id.flurstuecksnummer(), "94/1");
        // Unbelegte Stellen werden kanonisch zu Füllzeichen: BW führt keine
        // Fluren (000 → ___) und keine Folgenummer (00 → __).
        assert_eq!(id.canonical(), "081460___000940001__");
    }

    #[test]
    fn schleswig_holstein_inspire_mit_fuellzeichen() {
        let id = ParcelId::parse("01253801600432______").unwrap();
        assert_eq!(id.bundesland(), Some("SH"));
        assert_eq!(id.gemarkung(), "2538");
        assert_eq!(id.flur().as_deref(), Some("16"));
        assert_eq!(id.zaehler(), "432");
        assert_eq!(id.nenner(), None);
        assert_eq!(id.flurstuecksnummer(), "432");
        assert_eq!(id.canonical(), "01253801600432______");
    }

    #[test]
    fn gemarkungsschluessel_deckt_sich_mit_gemaschl_der_quelle() {
        // NRW liefert im selben Feature flstkennz="05495803101089______"
        // und gemaschl="054958" — beide müssen zusammenpassen.
        let id = ParcelId::parse("05495803101089______").unwrap();
        assert_eq!(id.gemarkung_schluessel(), "054958");
        assert_eq!(id.flur().as_deref(), Some("31")); // Quelle: flur="031"
        assert_eq!(id.zaehler(), "1089"); // Quelle: flstnrzae="1089"
        assert_eq!(id.nenner(), None); // Feld fehlt im Quelldatensatz
        assert_eq!(id.bundesland(), Some("NW"));
    }

    #[test]
    fn thueringen_kennzeichen() {
        // Thüringen liefert weder gemaschl noch flstnrzae — alles muss aus
        // dem Kennzeichen rekonstruierbar sein.
        let id = ParcelId::parse("16010112600001______").unwrap();
        assert_eq!(id.bundesland(), Some("TH"));
        assert_eq!(id.gemarkung_schluessel(), "160101");
        assert_eq!(id.flur().as_deref(), Some("126"));
        assert_eq!(id.zaehler(), "1");
        assert_eq!(id.nenner(), None);
    }

    #[test]
    fn alphanumerischer_nenner() {
        // Echter Wert aus dem sächsischen Dienst: Der Nenner trägt einen
        // Buchstabenzusatz.
        let id = ParcelId::parse("140208___00952000a02").unwrap();
        assert_eq!(id.bundesland(), Some("SN"));
        assert_eq!(id.zaehler(), "952");
        assert_eq!(id.nenner(), Some("a".into()));
        assert_eq!(id.flurstuecksnummer(), "952/a");
        assert_eq!(id.canonical(), "140208___00952000a02");
    }

    #[test]
    fn sachsen_ohne_flur() {
        // Sachsen führt keine Fluren; die Stelle ist im Kennzeichen unbelegt.
        // Echter Wert, zu dem der Dienst flstnrzae=214 und flstnrnen=3 liefert.
        let id = ParcelId::parse("140208___00214000300").unwrap();
        assert_eq!(id.bundesland(), Some("SN"));
        assert_eq!(id.gemarkung_schluessel(), "140208");
        assert_eq!(id.flur(), None);
        assert_eq!(id.zaehler(), "214");
        assert_eq!(id.nenner(), Some("3".into()));
        assert_eq!(id.flurstuecksnummer(), "214/3");
    }

    #[test]
    fn kanonische_form_ist_idempotent() {
        for raw in [
            "11000172000506____",
            "08146000000094000100",
            "01253801600432______",
        ] {
            let once = ParcelId::parse(raw).unwrap().canonical();
            let twice = ParcelId::parse(&once).unwrap().canonical();
            assert_eq!(once, twice, "Rohwert {raw}");
            assert_eq!(once.len(), LEN_CANONICAL);
        }
    }

    #[test]
    fn berlin_und_kanonische_form_sind_dasselbe_flurstueck() {
        // Derselbe Datensatz, einmal 18- und einmal 20-stellig geschrieben:
        // beide müssen auf denselben Schlüssel abbilden, sonst schlägt die
        // Deduplizierung über Kachelgrenzen fehl.
        let short = ParcelId::parse("11000172000506____").unwrap();
        let long = ParcelId::parse("11000172000506______").unwrap();
        assert_eq!(short, long);
        assert_eq!(short.canonical(), long.canonical());
    }

    #[test]
    fn trennzeichen_werden_toleriert() {
        // Schreibweise mit Separatoren, wie sie in manchen Auszügen vorkommt.
        let id = ParcelId::parse("052632-001-00123/0000").unwrap();
        assert_eq!(id.bundesland(), Some("NW"));
        assert_eq!(id.gemarkung(), "2632");
        assert_eq!(id.flur().as_deref(), Some("1"));
        assert_eq!(id.zaehler(), "123");
        assert_eq!(id.nenner(), None);
        assert_eq!(id.flurstuecksnummer(), "123");
    }

    #[test]
    fn leerzeichen_gelten_als_fuellzeichen() {
        let id = ParcelId::parse("01253801600432      ").unwrap();
        assert_eq!(id.nenner(), None);
        assert_eq!(id.canonical(), "01253801600432______");
    }

    #[test]
    fn nullen_und_fuellzeichen_bedeuten_beide_kein_nenner() {
        // Baden-Württemberg schreibt "kein Nenner" als 0000, die übrigen Länder
        // als ____. Beobachtet an echten Werten: BW liefert
        // "08146000000634000000" für das Flurstück 634 (nicht 634/0).
        // Beide Schreibweisen müssen auf denselben Schlüssel abbilden.
        let fuellzeichen = ParcelId::parse("08146000000634____00").unwrap();
        let nullen = ParcelId::parse("08146000000634000000").unwrap();
        assert_eq!(fuellzeichen.nenner(), None);
        assert_eq!(nullen.nenner(), None);
        assert_eq!(fuellzeichen, nullen);
        assert_eq!(fuellzeichen.canonical(), nullen.canonical());
        assert_eq!(nullen.flurstuecksnummer(), "634");
    }

    #[test]
    fn echter_nenner_bleibt_erhalten() {
        // Gegenprobe: 0015 ist ein echter Nenner und darf nicht verschwinden.
        let id = ParcelId::parse("08146000003010001500").unwrap();
        assert_eq!(id.nenner(), Some("15".into()));
        assert_eq!(id.flurstuecksnummer(), "3010/15");
    }

    #[test]
    fn fehlerfaelle() {
        assert_eq!(ParcelId::parse("   "), Err(ParcelIdError::Empty));
        assert!(matches!(
            ParcelId::parse("123"),
            Err(ParcelIdError::BadLength { got: 3, .. })
        ));
        // Ein Wert mit Sonderzeichen bleibt unzulässig.
        assert!(matches!(
            ParcelId::parse("0814600000094000#100"),
            Err(ParcelIdError::BadChar { .. })
        ));
        // Land unbelegt → Pflichtfeld fehlt.
        assert!(matches!(
            ParcelId::parse("__146000000094000100"),
            Err(ParcelIdError::MissingField { field: "Land", .. })
        ));
    }

    #[test]
    fn unbekannter_laenderschluessel_liefert_none() {
        let id = ParcelId::parse("99146000000094000100").unwrap();
        assert_eq!(id.bundesland(), None);
    }
}
