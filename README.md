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
`ALKIS_PORT` in der `.env`. Compose startet zusätzlich Valkey mit LRU-Verdrängung
und `appendonly`, verbindet beides und wartet mit dem Dienst, bis der Cache
antwortet.

Das Image ist zweistufig gebaut: statisch gegen musl übersetzt, dann in ein
`distroless`-Image gelegt. Es enthält nur die Binärdatei — knapp 9 MB, keine
Shell, kein Paketmanager, und der Prozess läuft als `nonroot`. Ein CA-Bundle
braucht es nicht: reqwest ist auf rustls mit gebündeltem Wurzelspeicher
eingestellt.

Weil im Image keine Shell steckt, hat es bewusst keine `HEALTHCHECK`-Anweisung.
`GET /health` prüft man von außen.

### Ohne Docker

```bash
docker run -d --name alkis-valkey -p 6399:6379 valkey/valkey:8-alpine \
  valkey-server --maxmemory 512mb --maxmemory-policy allkeys-lru --appendonly yes

cargo build --release
ALKIS_BIND=127.0.0.1:8099 ALKIS_VALKEY_URL=redis://127.0.0.1:6399 \
  ./target/release/alkis-proxy
```

Valkey ist optional — ohne Cache läuft der Dienst langsamer, aber korrekt. Ob er
tatsächlich angebunden ist, sagt beim Start die Zeile `Cache angebunden`; sonst
steht dort eine Warnung. Der Port im `docker run` oben ist **6399**, nicht der
Valkey-Standardport — `ALKIS_VALKEY_URL` muss dazu passen, sonst läuft der Dienst
still ohne Cache weiter. Der URL-Schema-Präfix bleibt `redis://` — Valkey ist
protokollkompatibel zu Redis, es gibt kein eigenes Schema.

`RUST_LOG=alkis_proxy=debug` protokolliert jede eingehende Anfrage mitsamt
`bbox` und Client — das ist die Stelle, an der man sieht, welchen Ausschnitt
QGIS tatsächlich anfragt.

### Konfiguration

| Variable | Vorgabe | Bedeutung |
|---|---|---|
| `ALKIS_BIND` | `0.0.0.0:8080` | Adresse des HTTP-Servers |
| `ALKIS_VALKEY_URL` | – | Valkey-URL; fehlt sie, läuft der Dienst ohne Cache |
| `ALKIS_UPSTREAM_TIMEOUT_SECS` | `30` | Zeitlimit je Landesdienst |
| `ALKIS_DEFAULT_LIMIT` | `5000` | Vorgabe für `limit` |
| `ALKIS_MAX_LIMIT` | `5000` | Obergrenze für `limit` |
| `ALKIS_PUBLIC_URL` | – | Nach außen sichtbare Basis-URL; nur nötig hinter einem Reverse Proxy |
| `ALKIS_DISABLE_COMPRESSION` | – | Gesetzt und nicht leer: keine Antwortkompression |
| `RUST_LOG` | `alkis_proxy=info` | Protokollierung |

Nur für `docker compose`, nicht vom Dienst selbst gelesen:

| Variable | Vorgabe | Bedeutung |
|---|---|---|
| `ALKIS_PORT` | `8080` | Port auf dem Host; im Container immer 8080 |
| `ALKIS_CACHE_MAXMEMORY` | `512mb` | Speichergrenze des Valkey-Caches |

## Endpunkte

```
GET /collections/flurstuecke/items?bbox=<w>,<s>,<e>,<n>[&limit=][&offset=][&state=NW]
GET /collections/flurstuecke      Beschreibung inkl. Quellen und Lizenzen
GET /collections
GET /conformance                  Erfüllte Konformitätsklassen
GET /api                          API-Definition (OpenAPI 3.0)
GET /health                       Zustand, abgeschaltete Landesdienste
GET /metrics                      Prometheus
```

`bbox` in WGS84, **maximal etwa 8 km Kantenlänge**. Ohne `state` bestimmt der
Dienst die zuständigen Bundesländer selbst.

Die Grenze ergibt sich aus der Kachelzerlegung: Eine Anfrage darf höchstens 64
Kacheln des 1-km-Rasters berühren. Geprüft wird im nativen CRS des jeweiligen
Landes, also exakt so, wie gleich darauf abgerufen würde. Ein zu großer
Ausschnitt liefert **HTTP 400**, keine leere Antwort.

## Nutzung in QGIS

Zwei Wege, getestet mit QGIS 4.2.

### Dynamisch (empfohlen): OGC API - Features

1. **Layer → Datenquellenverwaltung → OGC API - Features**
2. **Neu** → Name frei wählen, URL `http://127.0.0.1:8099` → **OK** → **Verbinden**
3. Sammlung `Flurstücke (ALKIS)` auswählen → **Hinzufügen**

Setze das Projekt-Koordinatensystem auf `EPSG:4326`, damit die Flächen sichtbar werden.

#### Maßstabsgrenze setzen

Ist der Kartenausschnitt größer als etwa 8 km Kantengröße, antwortet der Dienst mit
HTTP 400. Damit QGIS erst gar nicht so weit anfragt, sollte am Layer eine
maßstabsabhängige Sichtbarkeit gesetzt sein:

**Layereigenschaften → Darstellung → Maßstabsabhängige Sichtbarkeit**,
Minimum etwa **1:50.000**.

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
