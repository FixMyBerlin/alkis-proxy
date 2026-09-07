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

## 4. OGC API Features + Redis-Cache ✅
- [x] `upstream/client.rs`, `fetch.rs` — reqwest-Pool, GetFeature, OGC-Exceptions
- [x] `cache/store.rs` — deadpool-redis, zstd
- [x] `cache/tile_cache.rs` — Read-Through, Truncation-Split, Dedupe
- [x] `api/` — /collections, /items, /items/{id}
- [x] Negative Ergebnisse cachen

## 5. Router, Resilienz, Observability ✅
- [x] Bundesland-Routing über Hüllboxen (statt VG250-Polygone: selbstkorrigierend, keine Geodaten nötig)
- [x] `upstream/breaker.rs` — Circuit Breaker pro Land
- [x] `/health`, `/metrics` (Prometheus)
- [x] Teilergebnisse mit `warnings` statt hartem Fehler

## 6. Docker  ⏸ (auf Wunsch zurückgestellt)
- [ ] Multi-Stage-Dockerfile (musl-static → distroless)
- [ ] docker-compose.yml (Service + Redis, LRU + appendonly)
- [ ] ENV-Konfiguration

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

## Offene Punkte
- [ ] Bayern: INSPIRE-WFS-Endpunkt verifizieren
- [ ] Attribution je Land zusammentragen (DL-DE BY 2.0 / Zero 2.0 / CC BY 4.0)
- [ ] Saarland: 5xx-Verhalten beobachten
