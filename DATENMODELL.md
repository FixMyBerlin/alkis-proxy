# Datenmodell: was belegt ist und was fehlt

Das Ausgabeschema ist für alle Länder gleich, die **Belegung** ist es nicht.
Nicht belegbare Felder sind `null`, nie abwesend.

## Immer belegt

In allen 15 angebundenen Ländern: `parcelId`, `parcelIdSource`, `bundesland`,
`gemarkungSchluessel`, `zaehler`, `flurstuecksnummer`, `flaecheQm`, Geometrie.

## Lücken je Land

| Land | Schema | gemarkungName | flur | gemeindeSchluessel | gemeindeName | kreisName | lagebezeichnung | nutzung | stand |
|---|---|:-:|:-:|:-:|:-:|:-:|:-:|:-:|:-:|
| SH | Inspire | — | ✓ | — | — | — | ✓ | — | ✓ |
| HH | Ave | ✓ | — | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| NI | Ave | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| HB | Ave (NAS) | — | ✓ | — | ✓ | ✓ | — | — | ✓ |
| NW | Ave | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| HE | Ave | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | — | ✓ |
| RP | Ave | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | — |
| BW | BwNora | ✓ | — | ✓ | ✓ | — | — | — | ✓ |
| SL | Ave | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| BE | Berlin | ✓ | ✓ | ✓ | ✓ | — | — | — | ✓ |
| BB | Ave | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | — | ✓ |
| MV | Ave | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| SN | Ave | ✓ | — | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| ST | Ave | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| TH | Ave | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |

Bayern ist nicht angebunden.

Am häufigsten fehlt **`nutzung`** — in SH, HB, HE, BW, BE und BB.

`flur` fehlt in HH, BW und SN, weil diese Länder **keine Flureinteilung führen**:
Weder das Kennzeichen noch ein Quellfeld trägt dort eine Flurnummer, direkt an
den Landesdiensten geprüft.

`lagebezeichnung` bleibt bei BW und BE **absichtlich** leer. Das naheliegende
Feld enthält dort etwas anderes: `flurstueckstext` ist die Flurstücksnummer,
`bezeich` der Objekttyp `"AX_Flurstueck"`.

`nenner` ist naturgemäß oft `null` — ein Flurstück „94" hat keinen Nenner, nur
„94/1" hat einen.

## Nicht enthalten, in keinem Land

**Eigentümer, Anschrift, Grundbuchblatt und Buchungsstelle** sind
ALKIS-Bestandsdaten und in keinem offenen WFS enthalten. Ebenso wenig
Bodenrichtwerte oder Gebäude — dafür gibt es eigene Dienste.

## Bewusst nicht übernommen

Aus dem Kennzeichen abgeleitet statt aus Einzelfeldern: `gemaschl`, `flurschl`,
`flstnrzae`/`flstnrnen`, `zae`/`nen`, `zaehler`/`nenner`, `gemarkung_id`.

Darüber hinaus verworfen:

| Schema | Felder |
|---|---|
| Ave | `regbezirk`, `regbezschl`, `kreisschl`, `landschl`, `land`, `oid`, `idflurst`, `abwrecht` |
| Inspire | `referencePoint`, `zoning`, `endLifespanVersion`, `validTo`, `namespace` |
| BwNora | `ist_gebucht`, `vorgaenger`, `nachfolger`, `anlass`, `ende`, `folgenummer`, `fno_verfahren`, `gml_id` |
| Berlin | `uuid`, `dst`, `fln`, `gmk`, `bezeich` |

Erweiterbar wären am ehesten `kreisSchluessel` (in 11 Ländern vorhanden) und der
INSPIRE-`referencePoint` als amtlicher Beschriftungspunkt.

## Messgrundlage

Stand 8. September 2026. Je Land 200 Flurstücke aus einem Ausschnitt in
bebautem Gebiet (Testpunkte aus `examples/live.rs`), RP 195. „✓" heißt „in
dieser Stichprobe durchgehend belegt" — bei `lagebezeichnung` kann die Quote
regional schwanken, weil nicht jedes Flurstück eine Adresse hat.

## Lizenz und Quellenvermerk je Land

Die Daten gehören den Ländern, nicht diesem Dienst. **Elf der fünfzehn Quellen
verlangen eine Namensnennung**; wer die Flurstücke veröffentlicht, muss den
Quellenvermerk der beteiligten Länder mitführen.

| Land | Lizenz | Quellenvermerk | Namensnennung |
|---|---|---|:-:|
| SH | CC BY 4.0 | © GeoBasis-DE/LVermGeo SH/CC BY 4.0 | **ja** |
| HH | DL-DE BY 2.0 | Freie und Hansestadt Hamburg, Landesbetrieb Geoinformation und Vermessung (LGV) | **ja** |
| NI | CC BY 4.0 | LGLN (Jahr) Creative Commons Namensnennung – 4.0 International (CC BY 4.0) | **ja** |
| HB | CC BY 4.0 | GeoBremen (Jahr) Creative Commons Namensnennung – 4.0 International (CC BY 4.0) | **ja** |
| NW | DL-DE Zero 2.0 | © GeoBasis-DE/NRW | — |
| HE | DL-DE Zero 2.0 | © GeoBasis-DE/HVBG | — |
| RP | DL-DE BY 2.0 | © GeoBasis-DE / LVermGeoRP (Jahr), dl-de/by-2-0 | **ja** |
| BW | DL-DE BY 2.0 | LGL-BW (Jahr) Datenlizenz Deutschland - Namensnennung - Version 2.0 | **ja** |
| SL | DL-DE BY 2.0 | © GeoBasis DE/LVGL-SL (Jahr) | **ja** |
| BE | DL-DE Zero 2.0 | © GeoBasis-DE/Berlin | — |
| BB | DL-DE BY 2.0 | © GeoBasis-DE/LGB, dl-de/by-2-0 | **ja** |
| MV | CC BY 4.0 | © GeoBasis-DE/M-V/CC BY 4.0 | **ja** |
| SN | DL-DE BY 2.0 | GeoSN, dl-de/by-2-0 | **ja** |
| ST | DL-DE BY 2.0 | © GeoBasis-DE / LVermGeo ST, dl-de/by-2-0 | **ja** |
| TH | DL-DE BY 2.0 | © GDI-Th, dl-de/by-2-0 | **ja** |

„(Jahr)" ist das Jahr des Datenbezugs — der Dienst setzt es beim Ausliefern ein.

Abschreiben muss man nichts: Jede `items`-Antwort trägt ein Feld `attribution`
mit genau den Ländern, aus denen ihre Flurstücke stammen.
`GET /collections/flurstuecke` nennt unter `sources` denselben Vermerk je Land,
dazu Lizenz, Lizenz-URL und ob die Namensnennung zwingend ist.

Der Zusatz **„(Daten bearbeitet)"** ist kein Zierrat: Der Proxy gibt die
Geometrien nicht unverändert weiter, sondern rechnet sie aus dem nativen UTM
nach WGS84 um und rundet auf sieben Nachkommastellen. DL-DE BY 2.0 und
CC BY 4.0 verlangen beide einen Hinweis darauf. Bei den Zero-Ländern entfällt er.

Zwei Stolperstellen, die beim Abgleich auffielen:

- **Bremen** liefert über den LGLN-Host, der Rechteinhaber ist aber
  ausdrücklich „GeoBremen" — nicht LGLN.
- **Hessen** und **Nordrhein-Westfalen** stehen unter DL-DE **Zero**: Eine
  Namensnennung ist dort nicht vorgeschrieben. Der Dienst führt sie trotzdem
  mit, weil das Nennen der Quelle guter Stil bleibt.

### Herkunft dieser Angaben

Sie stehen nicht auf Verdacht hier, sondern stammen aus `ows:Fees` und
`ows:AccessConstraints` der GetCapabilities-Antworten der Dienste selbst. Zwei
Länder nennen dort keine Lizenz: Hamburg gibt nur den Quellenvermerk an, der
Saarland-Dienst (ArcGIS) gar nichts — für beide stammt sie aus dem
Metadatensatz beziehungsweise der Geobasisdatenübersicht des Landes. Geprüft am
8. September 2026, zusammen mit der Belegungsmessung oben.
