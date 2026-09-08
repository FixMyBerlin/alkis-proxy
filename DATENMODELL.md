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
