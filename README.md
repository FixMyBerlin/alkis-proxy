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

Im Hintergrund, mit Protokoll in eine Datei:

```bash
setsid env ALKIS_BIND=127.0.0.1:8099 ALKIS_VALKEY_URL=redis://127.0.0.1:6399 \
  RUST_LOG=alkis_proxy=debug,tower_http=debug \
  ./target/release/alkis-proxy >> /tmp/alkis.log 2>&1 < /dev/null &
```

Neu bauen und ersetzen:

```bash
pkill -f 'target/release/alkis-proxy'
cargo build --release
# ... dann wie oben starten
```

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
| `ALKIS_MAX_LIMIT` | `10000` | Obergrenze für `limit` |
| `RUST_LOG` | `alkis_proxy=info` | Protokollierung |

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

`bbox` in WGS84, **maximal etwa 16 km Kantenlänge**. Ohne `state` bestimmt der
Dienst die zuständigen Bundesländer selbst.

Die Grenze ergibt sich aus der Kachelzerlegung: Eine Anfrage darf höchstens 256
Kacheln des 1-km-Rasters berühren. Geprüft wird im nativen CRS des jeweiligen
Landes, also exakt so, wie gleich darauf abgerufen würde. Ein zu großer
Ausschnitt liefert **HTTP 400**, keine leere Antwort — warum das wichtig ist,
steht unter [Nutzung in QGIS](#nutzung-in-qgis).

Enthält ein Ausschnitt mehr Flurstücke, als `limit` zulässt, verweist die
Antwort über einen `next`-Link auf die nächste Seite; `numberMatched` nennt die
Gesamtzahl der Treffer. Ein Ausschnitt von wenigen Quadratkilometern kann in
dicht bebautem Gebiet mehrere tausend Flurstücke enthalten — Kölner Innenstadt,
3 × 2 km: rund 8700.

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
Ausschnitt nach.

#### Maßstabsgrenze setzen — nicht optional

Ist der Kartenausschnitt größer als etwa 16 km, antwortet der Dienst mit
HTTP 400. Damit QGIS erst gar nicht so weit anfragt, sollte am Layer eine
maßstabsabhängige Sichtbarkeit gesetzt sein:

**Layereigenschaften → Darstellung → Maßstabsabhängige Sichtbarkeit**,
Minimum etwa **1:50.000**.

Ohne diese Einstellung erscheint beim Herauszoomen eine Fehlermeldung in der
Meldungsleiste. Das ist unschön, aber harmlos — und deutlich besser als die
Alternative, siehe den nächsten Abschnitt.

#### Warum ein Fehler und keine leere Antwort

QGIS führt im OAPIF-Provider einen eigenen Kartencache und merkt sich, welche
Regionen es **vollständig** geladen hat. Beantwortet der Dienst einen zu weiten
Ausschnitt mit `HTTP 200` und einer leeren `FeatureCollection`, trägt QGIS die
gesamte Region als „geladen, enthält nichts" ein. Danach fragt es für jeden
Ausschnitt *innerhalb* dieser Region nicht mehr nach — auch nicht nach dem
Hineinzoomen. Der Layer bleibt leer, bis er neu angelegt wird.

Genau das war das Symptom „einmal herausgezoomt und wieder hinein, und es
kommt nichts mehr". Nachgemessen mit PyQGIS, gleicher Nachbarausschnitt:

| Ablauf | gelieferte Objekte |
|---|---|
| hineinzoomen → Nachbarausschnitt | 608 |
| hineinzoomen → **herauszoomen** → Nachbarausschnitt (leere 200er-Antwort) | 72 |
| hineinzoomen → **herauszoomen** → Nachbarausschnitt (400er-Antwort) | 608 |

Ein Fehlerstatus hinterlässt diesen Eintrag nicht, deshalb antwortet der Dienst
so. Der Cache liegt im QGIS-Prozess, nicht im Dienst — deshalb half früher nur,
den Layer neu hinzuzufügen.

#### „Ich sehe nichts, aber die Objektabfrage zeigt Geometrien"

Dasselbe vergiftete Cache-Fenster. Beide Werkzeuge fragen verschieden große
Ausschnitte an:

* Das **Zeichnen** fragt den ganzen Kartenausschnitt an — der lag über der
  Grenze und lieferte leer.
* Die **Objektabfrage** fragt nur einen kleinen Radius um den Mausklick an —
  der blieb darunter und lieferte.

Deshalb wirkte es, als sei nur die Objektabfrage funktionsfähig und „das
Styling kaputt".

**Zum Projekt-KBS:** Dass ein Wechsel auf EPSG:4326 die Darstellung
zurückbringt, liegt nicht am Koordinatensystem selbst. Ein Wechsel des
Projekt-KBS baut die Layer neu auf und leert dabei denselben Kartencache — er
wirkt also wie „Layer neu hinzufügen". Mit vorgewärmtem Cache zeichnet der
Layer in EPSG:4326, 3857, 25832 und 25833 gleich gut; nachgemessen.

Einen echten, wenn auch kleineren Effekt hat das Projekt-KBS trotzdem: QGIS
rechnet den Kartenausschnitt ins Layer-KBS zurück und nimmt dabei die Hüllbox
der umprojizierten Fläche. Die ist etwas größer als das Original — bei
EPSG:25832 rund 2 %, bei **EPSG:25833 rund 26 %**. Wer weit östlich in Zone 33
arbeitet, stößt also früher an die 16-km-Grenze als die Karte vermuten lässt.

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
cargo test                    # 108 Unit- + 8 Integrationstests
cargo run --example demo      # Harmonisierung an echten Fixtures
cargo run --example live      # alle Landesdienste live prüfen
```

Die Fixtures unter `tests/fixtures/` sind echte Antworten der Landesdienste.
