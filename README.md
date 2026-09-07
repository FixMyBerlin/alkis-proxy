# alkis-proxy

Flurstücke aus den Liegenschaftskatastern **aller deutschen Bundesländer** über
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

```bash
docker run -d --name alkis-redis -p 6399:6379 redis:7-alpine \
  redis-server --maxmemory 512mb --maxmemory-policy allkeys-lru --appendonly yes

cargo build --release
ALKIS_BIND=127.0.0.1:8099 ALKIS_REDIS_URL=redis://127.0.0.1:6399 \
  ./target/release/alkis-proxy
```

Redis ist optional — ohne Cache läuft der Dienst langsamer, aber korrekt.

### Konfiguration

| Variable | Vorgabe | Bedeutung |
|---|---|---|
| `ALKIS_BIND` | `0.0.0.0:8080` | Adresse des HTTP-Servers |
| `ALKIS_REDIS_URL` | – | Redis-URL; fehlt sie, läuft der Dienst ohne Cache |
| `ALKIS_UPSTREAM_TIMEOUT_SECS` | `30` | Zeitlimit je Landesdienst |
| `ALKIS_DEFAULT_LIMIT` | `5000` | Vorgabe für `limit` |
| `ALKIS_MAX_LIMIT` | `10000` | Obergrenze für `limit` |
| `RUST_LOG` | `alkis_proxy=info` | Protokollierung |

## Endpunkte

```
GET /collections/flurstuecke/items?bbox=<w>,<s>,<e>,<n>[&limit=][&state=NW]
GET /collections/flurstuecke      Beschreibung inkl. Quellen und Lizenzen
GET /collections
GET /health                       Zustand, abgeschaltete Landesdienste
GET /metrics                      Prometheus
```

`bbox` in WGS84, maximal 1° Kantenlänge. Ohne `state` bestimmt der Dienst die
zuständigen Bundesländer selbst.

## Nutzung in QGIS

Zwei Wege, beide getestet mit QGIS 4.2.

### Dynamisch (empfohlen): OGC API - Features

1. **Layer → Datenquellenverwaltung → OGC API - Features**
2. **Neu** → Name frei wählen, URL `http://127.0.0.1:8099` → **OK** → **Verbinden**
3. Sammlung `flurstuecke` auswählen → **Hinzufügen**

**Wichtig:** In den Verbindungseinstellungen **„Nur Objekte anfordern, die den
aktuellen Kartenausschnitt überschneiden"** aktivieren. Ohne diese Option fragt
QGIS ohne Ausschnitt an und bekommt nur ein Beispiel-Sample.

Der Layer lädt dann beim Verschieben und Zoomen jeweils den sichtbaren
Ausschnitt nach. Oberhalb von etwa 1 Grad Kantenlänge bleibt er leer — das ist
gewollt, sonst würde ein einziger Kartenausschnitt tausende Abfragen auslösen.
Sinnvoll ist deshalb eine maßstabsabhängige Darstellung ab etwa 1:25.000.

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

Nicht belegbare Felder sind `null`, nie abwesend. `flur` ist in Baden-Württemberg,
Sachsen und Hamburg immer `null` — diese Länder führen keine Flureinteilung.

## Wie die Vereinheitlichung funktioniert

**Vier Quellschemata**, nicht sechzehn: AdV „ALKIS vereinfacht" (11 Länder),
INSPIRE (SH, SL), Baden-Württemberg (`nora`), Berlin. Das Saarland liefert AVE
in Großbuchstaben, Bremen unter NAS-Feldnamen — beides deckt derselbe Adapter ab.

**Das Kennzeichen ist die Wahrheit.** Gemarkung, Flur, Zähler und Nenner werden
aus dem Flurstückskennzeichen abgeleitet, nicht aus Einzelfeldern: Thüringen
liefert weder `gemaschl` noch `flstnrzae`, INSPIRE kennt die Felder gar nicht.
Die Kennzeichen kommen 18- oder 20-stellig, mit `_`- oder Null-Auffüllung und
teils mit Buchstaben (`140208___00952000a02`) — sie werden auf eine kanonische
20-Zeichen-Form gebracht.

**Native Abfrage statt EPSG:4326.** Angefragt wird immer im UTM-CRS des Landes.
In 4326 ist die Achsenreihenfolge unter WFS 2.0 uneinheitlich, und drei Länder
bieten es gar nicht an. Die Rückrechnung nach WGS84 macht der Dienst selbst.

**Kachelbasierter Cache.** Beliebige Bounding-Boxen wiederholen sich nie; der
Cache arbeitet deshalb auf einem 1-km-Raster im nativen CRS. Antworten, die
exakt am Abrufslimit liegen, gelten als abgeschnitten — solche Kacheln werden
geviertelt, und diese Erkenntnis wird mitgespeichert.

## Entwicklung

```bash
cargo test                    # 96 Unit- + 8 Integrationstests
cargo run --example demo      # Harmonisierung an echten Fixtures
cargo run --example live      # alle Landesdienste live prüfen
```

Die Fixtures unter `tests/fixtures/` sind echte Antworten der Landesdienste.
