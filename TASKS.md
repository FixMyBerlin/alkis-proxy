# ALKIS-Proxy — Aufgabenliste

Abgeleitet aus dem genehmigten Plan. Status: `[ ]` offen · `[x]` erledigt · `[~]` in Arbeit

## 1. Gerüst + Kennzeichen-Parser ✅
- [x] Cargo-Projekt anlegen, Modulstruktur
- [x] `model/parcel_id.rs` — Parser für 18/20-stellige Kennzeichen
- [x] Tests gegen die drei echten Werte (BE 18-stellig, BW 20-stellig, SH `_`-Padding)
- [x] Länderschlüssel → Bundesland-Kürzel (01=SH … 16=TH)
- [x] `model/parcel.rs` — harmonisiertes Zielschema
- [x] `config/states.rs` — Endpunkt-Katalog der 16 Länder

## 2. Adapter + GML-Parser ✅
- [x] `adapters/mod.rs` — Dispatch über `SchemaVariant` (statt Trait: Mapper sind zustandslos, Menge geschlossen)
- [x] `parse/gml.rs` — quick-xml Streaming (Zielfelder + `gml:posList`)
- [x] `parse/geojson.rs` — Direktpfad für BE/BW/RP
- [x] Polygon → MultiPolygon-Hochstufung (in gml.rs/geojson.rs statt eigenem Modul)
- [x] `adapters/ave.rs` (11 Länder)
- [x] `adapters/inspire.rs` (SH, SL, BY?)
- [x] `adapters/bw_nora.rs`
- [x] `adapters/berlin.rs`
- [x] Fixtures aus echten Antworten + Adapter-Tests

## 3. CRS + native BBOX ✅
- [x] `crs/reproject.rs` — eigene UTM-Transformation (statt proj4rs: keine native Abhängigkeit, gegen PROJ verifiziert)
- [x] `crs/tiles.rs` — 1-km-Kachelraster, BBOX ↔ Kachelliste
- [x] Verifikation: MV, SN, TH liefern über den nativen Pfad Features (live geprüft)

## 4. OGC API Features + Valkey-Cache ✅
- [x] `upstream/client.rs`, `fetch.rs` — reqwest-Pool, GetFeature, OGC-Exceptions
- [x] `cache/store.rs` — deadpool-redis (RESP-kompatibel zu Valkey), zstd
- [x] `cache/tile_cache.rs` — Read-Through, Truncation-Split, Dedupe
- [x] `api/` — /collections, /items, /items/{id}
- [x] Negative Ergebnisse cachen

## 5. Router, Resilienz, Observability ✅
- [x] Bundesland-Routing über Hüllboxen (statt VG250-Polygone: selbstkorrigierend, keine Geodaten nötig)
- [x] `upstream/breaker.rs` — Circuit Breaker pro Land
- [x] `/health`, `/metrics` (Prometheus)
- [x] Teilergebnisse mit `warnings` statt hartem Fehler

## 6. Docker ✅
- [x] Multi-Stage-Dockerfile (musl-static → distroless), Image 8,3 MB, `nonroot`
- [x] docker-compose.yml (Service + Valkey, LRU + appendonly, `service_healthy`)
- [x] ENV-Konfiguration + `.env.example`; README-Tabelle vervollständigt
- [x] `ALKIS_DISABLE_COMPRESSION`: leerer Wert zählt jetzt als nicht gesetzt —
      Compose setzt eine Variable ohne `.env`-Wert auf den leeren String und
      hätte die Kompression sonst ungewollt abgeschaltet
- Kein `HEALTHCHECK` im Image: distroless hat keine Shell, mit der sie liefe.
  `/health` wird von außen geprüft.

## 7. Audit-Binary  ⏸ (auf Wunsch zurückgestellt)
- [ ] `bin/audit.rs` — GetCapabilities + GetFeature-Probe je Land
- [ ] Testkoordinaten verifizieren (bebautes Gebiet)
- [ ] JSON-Report, Anbindung an /health

## Nach dem Bau gefundene Punkte (behoben)
- [x] Sachsen und Baden-Württemberg führen keine Fluren — `flur` ist optional
- [x] Kennzeichen können Buchstaben enthalten (`140208___00952000a02`)
- [x] Cache-Schlüssel brauchte das Bundesland: überlappende Hüllboxen ließen die
      leere Antwort eines Landes die Daten des Nachbarn verdecken
- [x] Saarland lehnt explizite OUTPUTFORMAT-Angaben ab (ArcGIS), Felder in Großschrift
- [x] Rheinland-Pfalz liefert GML, nicht JSON
- [x] Bremen nutzt NAS-Feldnamen (`flurstueckskennzeichen`, `amtlicheFlaeche`)
- [x] Ausgabekoordinaten auf 7 Nachkommastellen gerundet (~1 cm, ein Drittel kleinere Antworten)

## OGC-Konformität: was QGIS am Dienst scheitern ließ (behoben)

Gefunden durch Abgleich von OGC API Features Teil 1 mit dem Quelltext des
QGIS-OAPIF-Providers (`QgsOapifProvider::init`, `QgsOapifLandingPageRequest`).

- [x] `service-desc` zeigte auf die Sammlung statt auf eine API-Definition
      (Verstoß gegen Requirement 2). QGIS fand darin keine Seitengröße und fiel
      auf `mPageSize = 100` zurück
- [x] `/api` ergänzt — minimales OpenAPI 3.0. QGIS liest daraus
      `components.parameters.limit.schema.{default,maximum}` und wählt seither
      5000 statt 100
- [x] Kein `next`-Link: der Layer endete nach der ersten Seite. Ein Kölner
      Ausschnitt lieferte 100 von 8723 Flurstücken. Jetzt Seitennavigation über
      `offset` mit `next`/`prev`
- [x] `numberMatched` war gleich `numberReturned`, statt die Treffermenge zu
      nennen; Clients konnten Abschneidung nicht erkennen. Wird jetzt über die
      volle Menge berechnet und weggelassen, wo sie unbekannt ist
- [x] Das Schema-Sample (Anfrage ohne bbox) meldete `numberMatched: 2` — QGIS
      übernahm das per `setFeatureCount(…, exact)` als Größe des ganzen Layers.
      Das Sample nennt jetzt keine Treffermenge mehr
- [x] `items` antwortete als `application/json` statt `application/geo+json`,
      `queryables` jetzt als `application/schema+json`
- [x] Konformitätsklasse `conf/oas30` ergänzt
- [x] Kommentar zu `/queryables` richtiggestellt: QGIS ruft den Endpunkt nur bei
      CQL2-Filterung ab, die Felder stammen aus dem Sample

## Offene Punkte zur Konformität
- [ ] Teil 5 (Schemas): würde die Feldliste unabhängig vom Sample machen.
      Zurückgestellt — QGIS ersetzt damit `mFields` vollständig, und das Format
      ist noch ein Entwurf

## Offene Punkte
- [ ] Bayern: INSPIRE-WFS-Endpunkt verifizieren
- [x] Attribution je Land zusammentragen — alle 15 Länder belegt, Quellen sind
      `ows:Fees`/`ows:AccessConstraints` der Dienste (HH und SL aus dem
      Metadatensatz, weil ihre Capabilities nichts nennen). 11 Länder verlangen
      Namensnennung, 4 stehen unter Zero. Bremen korrigiert: Rechteinhaber ist
      „GeoBremen", nicht LGLN. Neu: `attribution` in jeder `items`-Antwort,
      beschränkt auf die Länder, aus denen die Features stammen
- [ ] Saarland: 5xx-Verhalten beobachten
