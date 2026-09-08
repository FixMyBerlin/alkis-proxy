# alkis-proxy

Flurstücke aus den Liegenschaftskatastern **aller deutschen Bundesländer (außer Bayern)** über
eine einheitliche OGC-API-Features-Schnittstelle. Der Dienst verbirgt, dass
dahinter 15 verschiedene WFS mit vier Datenschemata, zwei Projektionen und
unterschiedlichen Ausgabeformaten stecken.

Ausgabe ist immer **GeoJSON in EPSG:4326** mit einem festen Attributsatz.

## Stand

15 von 16 Bundesländern sind angebunden und liefern nachweislich Daten. Für
**Bayern** ist kein kostenfreier Endpunkt konfiguriert: Der WFS über
GeodatenOnline ist gebührenpflichtig; der INSPIRE-Downloaddienst ist der
aussichtsreichste Kandidat und noch zu prüfen.

## Starten

### Mit Docker

```bash
cp .env.example .env      # optional, alle Werte haben Vorgaben
docker compose up -d
```

Danach liegt der Dienst auf <http://127.0.0.1:8080>; ein anderer Port geht über
`ALKIS_PORT` in der `.env`. Der Cache läuft eingebettet im Dienst selbst
(redb, eine Datei unter `/data` im Container)

Das Image ist zweistufig gebaut: statisch gegen musl übersetzt, dann in ein
`distroless`-Image gelegt. Es enthält nur die Binärdatei — knapp 9 MB, keine
Shell, kein Paketmanager, und der Prozess läuft als `nonroot`. Ein CA-Bundle
braucht es nicht: reqwest ist auf rustls mit gebündeltem Wurzelspeicher
eingestellt.

Weil im Image keine Shell steckt, hat es bewusst keine `HEALTHCHECK`-Anweisung.
`GET /health` prüft man von außen.

### Ohne Docker

```bash
cargo build --release
ALKIS_BIND=127.0.0.1:8099 ALKIS_CACHE_PATH=./cache.redb ALKIS_CACHE_MAX_SIZE=2gb \
  ALKIS_OSM_PBF_PATH=./data/baden-wuerttemberg-latest.osm.pbf \
  ./target/release/alkis-proxy
```

Der Cache ist optional — ohne `ALKIS_CACHE_PATH` läuft der Dienst langsamer, aber
korrekt, ganz ohne weiteren Prozess. Ob er tatsächlich angebunden ist, sagt beim
Start die Zeile `Cache angebunden`; sonst steht dort eine Warnung. Die Datei
unter `ALKIS_CACHE_PATH` legt der Dienst selbst an, das Verzeichnis muss aber
existieren und beschreibbar sein.

`ALKIS_OSM_PBF_PATH` ist ebenso optional und aktiviert die zusätzliche
Collection `flurstuecke-nutzungsart` (siehe [unten](#optional-nutzungsart-collection-flurstuecke-nutzungsart)) —
ohne die Variable läuft der Dienst unverändert, ganz ohne die zweite Sammlung.

`RUST_LOG=alkis_proxy=debug` protokolliert jede eingehende Anfrage mitsamt
`bbox` und Client — das ist die Stelle, an der man sieht, welchen Ausschnitt
QGIS tatsächlich anfragt.

### Konfiguration

| Variable | Vorgabe | Bedeutung |
|---|---|---|
| `ALKIS_BIND` | `0.0.0.0:8080` | Adresse des HTTP-Servers |
| `ALKIS_CACHE_PATH` | – | Pfad der redb-Cache-Datei; fehlt er, läuft der Dienst ohne Cache |
| `ALKIS_CACHE_MAX_SIZE` | `2gb` | Obergrenze der Cache-Datei (`512mb`, `32gb`, oder Byte-Zahl); bei Überschreiten weichen die ältesten Kacheln |
| `ALKIS_UPSTREAM_TIMEOUT_SECS` | `30` | Zeitlimit je Landesdienst |
| `ALKIS_DEFAULT_LIMIT` | `5000` | Vorgabe für `limit` |
| `ALKIS_MAX_LIMIT` | `5000` | Obergrenze für `limit` |
| `ALKIS_PUBLIC_URL` | – | Nach außen sichtbare Basis-URL; nur nötig hinter einem Reverse Proxy |
| `ALKIS_DISABLE_COMPRESSION` | – | Gesetzt und nicht leer: keine Antwortkompression |
| `ALKIS_OSM_PBF_PATH` | – | Pfad einer lokalen OSM-PBF-Datei; aktiviert die optionale Collection `flurstuecke-nutzungsart` (siehe unten) |
| `RUST_LOG` | `alkis_proxy=info` | Protokollierung |

Nur für `docker compose`, nicht vom Dienst selbst gelesen:

| Variable | Vorgabe | Bedeutung |
|---|---|---|
| `ALKIS_PORT` | `8080` | Port auf dem Host; im Container immer 8080 |

## Endpunkte

```
GET /collections/flurstuecke/items?bbox=<w>,<s>,<e>,<n>[&limit=][&offset=][&state=NW]
GET /collections/flurstuecke      Beschreibung inkl. Quellen und Lizenzen
GET /collections/flurstuecke-nutzungsart/items?bbox=...   nur mit ALKIS_OSM_PBF_PATH
GET /collections/flurstuecke-nutzungsart
GET /collections
GET /conformance                  Erfüllte Konformitätsklassen
GET /api                          API-Definition (OpenAPI 3.0)
GET /health                       Zustand, abgeschaltete Landesdienste, Nutzungsart-Index
GET /metrics                      Prometheus
```

`bbox` in WGS84, **maximal etwa 8 km Kantenlänge**. Ohne `state` bestimmt der
Dienst die zuständigen Bundesländer selbst.

Die Grenze ergibt sich aus der Kachelzerlegung: Eine Anfrage darf höchstens 64
Kacheln des 1-km-Rasters berühren. Geprüft wird im nativen CRS des jeweiligen
Landes, also exakt so, wie gleich darauf abgerufen würde. Ein zu großer
Ausschnitt liefert **HTTP 400**, keine leere Antwort.

Eine zweite Grenze gilt unabhängig von der Fläche: Enthält ein Ausschnitt mehr
Flurstücke als `ALKIS_MAX_LIMIT` (Vorgabe 5000), wird er ebenfalls mit **HTTP
400** abgelehnt statt gekürzt geliefert. In dicht bebautem Gebiet reichen dafür
schon anderthalb Kilometer Kantenlänge — Stuttgart-Ost, 1,5 × 2,2 km: rund 5450
Flurstücke. Der Grund ist derselbe wie oben: Eine gekürzte Antwort trägt keinen
`next`-Link, weil der Dienst die fehlenden Flurstücke gar nicht kennt, und ist
damit von einer vollständigen nicht zu unterscheiden. Wer mehr auf einmal
braucht und den Speicher hat, setzt `ALKIS_MAX_LIMIT` hoch.

`numberMatched` nennt deshalb immer die vollständige Treffermenge — außer beim
Schema-Sample, das ohne `bbox` geliefert wird.

## Nutzung in QGIS

Zwei Wege, getestet mit QGIS 4.2.

### Dynamisch (empfohlen): OGC API - Features

1. **Layer → Datenquellenverwaltung → OGC API - Features**
2. **Neu** → Name frei wählen, URL `http://127.0.0.1:8099` → **OK** → **Verbinden**
3. Sammlung `Flurstücke (ALKIS)` auswählen → **Hinzufügen**

Setze das Projekt-Koordinatensystem auf `EPSG:4326`, damit die Flächen sichtbar werden.

#### Maßstabsgrenze setzen

Ist der Kartenausschnitt größer als etwa 8 km Kantengröße — oder enthält er mehr
als `ALKIS_MAX_LIMIT` Flurstücke —, antwortet der Dienst mit HTTP 400. Damit
QGIS erst gar nicht so weit anfragt, sollte am Layer eine maßstabsabhängige
Sichtbarkeit gesetzt sein:

**Layereigenschaften → Darstellung → Maßstabsabhängige Sichtbarkeit**,
Minimum etwa **1:50.000** — in Innenstädten eher **1:10.000**, sonst greift die
Flurstücksgrenze.

Ohne diese Einstellung erscheint beim Herauszoomen eine Fehlermeldung in der
Meldungsleiste. Das ist unschön, aber harmlos — und deutlich besser als die
Alternative, siehe den nächsten Abschnitt.

### Statisch: als GeoJSON-Layer

Für ein festes Projektgebiet genügt ein Vektorlayer mit der Item-URL:

```
http://127.0.0.1:8099/collections/flurstuecke/items?bbox=9.176,48.774,9.180,48.778&limit=2000
```

**Layer → Layer hinzufügen → Vektorlayer → Protokoll: HTTP(S)** und diese URL
eintragen. Lädt einmalig, dafür ohne Nachladen beim Zoomen.

## Ausgabeschema

```json
{
  "id": "NW:05495803101089______",
  "properties": {
    "parcelId": "05495803101089______",   "parcelIdSource": "flstkennz",
    "bundesland": "NW",
    "gemarkungSchluessel": "054958",      "gemarkungName": "Köln",
    "flur": "31",                          "zaehler": "1089",
    "nenner": null,                        "flurstuecksnummer": "1089",
    "flaecheQm": 9.0,
    "gemeindeSchluessel": "05315000",     "gemeindeName": "Köln",
    "kreisName": "Köln",
    "lagebezeichnung": "Marspfortengasse 6",
    "nutzung": "Fläche gemischter Nutzung",
    "stand": "2013-02-22"
  }
}
```

Welche Felder in welchem Land belegt sind, welche Angaben in keinem Dienst
enthalten sind und was die Adapter bewusst verwerfen, steht in [DATENMODELL.md](DATENMODELL.md).

## Optional: Nutzungsart-Collection (`flurstuecke-nutzungsart`)

Die öffentlichen ALKIS-Daten enthalten aus Datenschutzgründen keine
Eigentümerangaben. `flurstuecke-nutzungsart` liefert dieselben Flurstücke wie
`flurstuecke`, ergänzt um eine **geschätzte** Nutzungsart — **ausschließlich
aus OpenStreetMap** abgeleitet, nie aus ALKIS-Zusatzquellen:

```json
{
  "properties": {
    "...": "alle Felder von flurstuecke, unverändert",
    "category": "privat",
    "rule": "wohngebaeude",
    "confidence": 0.6,
    "conflict": null
  }
}
```

| Feld | Bedeutung |
|---|---|
| `category` | `privat` \| `oeffentlich` \| `bahn` \| `unbekannt` |
| `rule` | Name der greifenden Regel (`src/classification/rules.rs`) |
| `confidence` | Konfidenz der Schätzung, 0.0–1.0 |
| `conflict` | Abweichende Kategorie einer anderen greifenden Regel, falls vorhanden |

Aktiviert wird die Collection über `ALKIS_OSM_PBF_PATH`, den Pfad einer
lokalen `.osm.pbf`-Datei (z. B. von [Geofabrik](https://download.geofabrik.de/europe/germany.html)).
Ohne diese Variable taucht `flurstuecke-nutzungsart` in `/collections` gar
nicht erst auf — der Dienst läuft unverändert weiter.

```bash
ALKIS_OSM_PBF_PATH=./baden-wuerttemberg-latest.osm.pbf ./target/release/alkis-proxy
```

Die Datei wird beim Start **einmalig** in einen residenten Index geladen
(Fortschritt sichtbar im Log, Endzustand über `GET /health` → Feld
`classification`: `aus` | `wird_indiziert` | `bereit`). Jede Bbox-Anfrage
klassifiziert danach live gegen diesen Index — es wird nichts vorberechnet
oder zwischengespeichert. Bis der Index fertig ist, liefert die Collection die
Flurstücke bereits, aber ohne Nutzungsart, dazu eine Warnung im Feld
`warnings` der Antwort.

Für eine deutschlandweite Datei reduziert ein Vorfilter mit
[osmium-tool](https://osmcode.org/osmium-tool/) die Startzeit erheblich —
optional, aber empfohlen:

```bash
osmium tags-filter germany-latest.osm.pbf \
  w/operator:type w/building=house,residential,detached,apartments,public,school \
  w/amenity=townhall,public_building,courthouse,community_centre,school \
  w/leisure=schoolyard,park,playground \
  w/landuse=industrial,commercial,railway \
  w/tourism=zoo,hotel w/railway=rail \
  -o germany-filtered.osm.pbf
```

**Bekannte Einschränkungen** (siehe Code-Kommentare in `src/classification/`):

- **Keine dedizierte Straßen-Regel.** Ohne amtlichen Verkehrsnetz-WFS (bewusst
  nicht verwendet — das wäre ALKIS, nicht OSM) und ohne Puffer um
  `highway=*`-Linien werden reine Straßenflurstücke meist als `unbekannt`
  klassifiziert statt als `oeffentlich`. Die größte bekannte Lücke von v1.
- **Nur einfache Ways, keine Multipolygon-Relationen.** Größere Parks und
  manche Landnutzungsflächen sind in OSM oft als Relation statt als Way
  gemappt und werden nicht erkannt.
- Kein Neuladen bei geänderter PBF-Datei zur Laufzeit — Neustart nötig.
- Die Klassifikation ist eine Heuristik mit Konfidenzwert, **keine verlässliche
  Eigentumsfeststellung**.

## Quellen und Lizenzen

Die Daten gehören den Ländern. Elf der fünfzehn Quellen verlangen eine
Namensnennung — wer die Flurstücke veröffentlicht, muss den Quellenvermerk der
beteiligten Länder mitführen.

Abschreiben muss man ihn nicht: Jede `items`-Antwort trägt ein Feld
`attribution` mit genau den Ländern, aus denen ihre Flurstücke stammen.

```bash
curl -s "http://127.0.0.1:8080/collections/flurstuecke/items?bbox=9.176,48.774,9.180,48.778" \
  | jq -r .attribution
# LGL-BW (2026) Datenlizenz Deutschland - Namensnennung - Version 2.0 (Daten bearbeitet)
```

Lizenz und geforderten Vermerk je Land führt
[DATENMODELL.md](DATENMODELL.md#lizenz-und-quellenvermerk-je-land) auf.

## Wie die Vereinheitlichung funktioniert

**Vier Quellschemata**: AdV „ALKIS vereinfacht" (12 Länder),
INSPIRE (SH), Baden-Württemberg (`nora`), Berlin. Das Saarland liefert AVE
in Großbuchstaben, Bremen unter NAS-Feldnamen — beides deckt derselbe Adapter ab.

**Native Abfrage statt EPSG:4326.** Angefragt wird immer im UTM-CRS des Landes.
Die Berechnung des CRS nach WGS84 erfolgt im Proxy.

**Kachelbasierter Cache.** Beliebige Bounding-Boxen wiederholen sich nie; der
Cache arbeitet deshalb auf einem 1-km-Raster im nativen CRS. Antworten, die
exakt am Abrufslimit liegen, gelten als abgeschnitten — solche Kacheln werden
geviertelt, und diese Erkenntnis wird mitgespeichert.

## Entwicklung

```bash
cargo test                    # 108 Unit- + 8 Integrationstests
cargo run --example demo      # Harmonisierung an echten Fixtures
cargo run --example live      # alle Landesdienste live prüfen
```

Die Fixtures unter `tests/fixtures/` sind echte Antworten der Landesdienste.
