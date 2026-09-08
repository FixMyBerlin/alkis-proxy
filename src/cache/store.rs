//! redb-Anbindung: Der Cache lebt eingebettet im Prozess als Datei auf der
//! Festplatte, nicht in einem separaten Dienst.
//!
//! Der Cache ist bewusst *optional*: Fehlt der Pfad oder lässt sich die
//! Datei nicht öffnen, arbeitet der Dienst ohne ihn weiter — langsamer, aber
//! korrekt. Ein Cache-Ausfall darf nicht zum Dienstausfall werden.
//!
//! Gespeichert werden zstd-komprimierte GeoJSON-Feature-Listen. Komprimieren
//! lohnt sich hier deutlich: Flurstücksgeometrien sind lange Zahlenreihen mit
//! viel Wiederholung, typisch bleiben unter 20 % der Rohgröße übrig.
//!
//! redb selbst kennt weder TTL noch eine Speichergrenze mit Verdrängung —
//! beides baut dieses Modul selbst nach: Ablauf über einen mitgespeicherten
//! Zeitstempel je Eintrag (`TILES`), Verdrängung über einen zweiten, nach
//! Einfügezeitpunkt sortierten Index (`AGE_INDEX`), sobald die Datei die
//! konfigurierte Grenze überschreitet. Verdrängt wird nach Einfügezeitpunkt,
//! nicht nach letztem Zugriff: echtes LRU würde bei jedem Cache-Treffer eine
//! zusätzliche Schreibtransaktion kosten — bei Kacheln, die wochenlang
//! unverändert bleiben, ist das den Aufwand nicht wert.
//!
//! redb ist synchron; jeder Zugriff läuft deshalb über `spawn_blocking`.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};

/// Kompressionsstufe. 3 ist der zstd-Standard: nahe am besten Verhältnis,
/// aber um ein Vielfaches schneller als die hohen Stufen.
const ZSTD_LEVEL: i32 = 3;

/// Kachel-Schlüssel → `[Ablaufzeit][Einfügezeit][zstd-Payload]`, siehe
/// [`decode_entry`].
const TILES: TableDefinition<&str, &[u8]> = TableDefinition::new("tiles");

/// `[Einfügezeit, big-endian][Kachel-Schlüssel]` → nichts. Die Big-Endian-
/// Kodierung sorgt dafür, dass Byte- und Zeitreihenfolge übereinstimmen —
/// der älteste Eintrag ist so per aufsteigendem Scan zu finden, ohne dass
/// die Werte selbst gelesen werden müssen.
const AGE_INDEX: TableDefinition<&[u8], ()> = TableDefinition::new("tiles_by_age");

/// Wie viele der ältesten Einträge ein Verdrängungslauf höchstens entfernt,
/// bevor die Dateigröße erneut geprüft wird.
const EVICTION_BATCH: usize = 500;

pub struct RedbStore {
    db: Arc<Database>,
    path: PathBuf,
    /// Obergrenze der Cache-Datei in Byte. 0 bedeutet unbegrenzt.
    max_bytes: u64,
}

impl RedbStore {
    /// Öffnet die Cache-Datei (legt sie bei Bedarf an) und richtet die
    /// beiden Tabellen ein.
    ///
    /// Schlägt das fehl, gibt der Aufrufer den Fehler weiter und der Dienst
    /// läuft ohne Cache.
    pub fn open(path: &Path, max_bytes: u64) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("Cache-Verzeichnis {}: {e}", parent.display()))?;
            }
        }

        let db = Database::create(path).map_err(|e| format!("redb: {e}"))?;
        let txn = db.begin_write().map_err(|e| format!("redb: {e}"))?;
        txn.open_table(TILES).map_err(|e| format!("redb: {e}"))?;
        txn.open_table(AGE_INDEX).map_err(|e| format!("redb: {e}"))?;
        txn.commit().map_err(|e| format!("redb: {e}"))?;

        Ok(RedbStore {
            db: Arc::new(db),
            path: path.to_path_buf(),
            max_bytes,
        })
    }

    /// Liest und entpackt einen Eintrag. Fehler und abgelaufene Einträge
    /// werden zu `None` — ein kaputter oder veralteter Cache-Eintrag soll
    /// die Anfrage nicht scheitern lassen, sondern nur einen Neuabruf
    /// auslösen.
    pub async fn get(&self, key: &str) -> Option<Vec<u8>> {
        let db = Arc::clone(&self.db);
        let key = key.to_string();
        tokio::task::spawn_blocking(move || get_blocking(&db, &key))
            .await
            .ok()
            .flatten()
    }

    /// Schreibt einen Eintrag mit Ablauffrist und verdrängt bei Bedarf die
    /// ältesten Einträge, falls die Datei dadurch über die Grenze wächst.
    /// Fehler werden nur geloggt.
    pub async fn set(&self, key: &str, value: &[u8], ttl: Duration) {
        let packed = match zstd::encode_all(value, ZSTD_LEVEL) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(key, error = %e, "Cache-Eintrag nicht komprimierbar");
                return;
            }
        };
        let db = Arc::clone(&self.db);
        let path = self.path.clone();
        let max_bytes = self.max_bytes;
        let key = key.to_string();
        let expires_at = now_secs() + ttl.as_secs();
        let inserted_at = now_millis();

        let result = tokio::task::spawn_blocking(move || {
            set_blocking(&db, &key, &packed, expires_at, inserted_at)?;
            evict_if_over_limit(&db, &path, max_bytes);
            Ok::<(), String>(())
        })
        .await;

        match result {
            Ok(Ok(())) => {}
            Ok(Err(e)) => tracing::warn!(error = %e, "Cache-Eintrag nicht speicherbar"),
            Err(e) => tracing::warn!(error = %e, "Cache-Schreibvorgang abgebrochen"),
        }
    }
}

fn get_blocking(db: &Database, key: &str) -> Option<Vec<u8>> {
    let txn = db.begin_read().ok()?;
    let table = txn.open_table(TILES).ok()?;
    let guard = table.get(key).ok().flatten()?;
    let (expires_at, inserted_at, payload) = decode_entry(guard.value())?;

    if expires_at < now_secs() {
        drop(guard);
        drop(table);
        drop(txn);
        remove_blocking(db, key, inserted_at);
        return None;
    }

    match zstd::decode_all(payload) {
        Ok(raw) => Some(raw),
        Err(e) => {
            tracing::warn!(key, error = %e, "Cache-Eintrag nicht entpackbar");
            None
        }
    }
}

fn set_blocking(
    db: &Database,
    key: &str,
    packed: &[u8],
    expires_at: u64,
    inserted_at: u64,
) -> Result<(), String> {
    let txn = db.begin_write().map_err(|e| e.to_string())?;
    {
        let mut tiles = txn.open_table(TILES).map_err(|e| e.to_string())?;
        let mut index = txn.open_table(AGE_INDEX).map_err(|e| e.to_string())?;

        // Überschreibt dieser Schlüssel einen bestehenden Eintrag (Ablauf
        // oder erneute Teilung einer Kachel), muss dessen alter Index-
        // Eintrag mit weg — sonst verweist er ins Leere, und die Verdrängung
        // räumt ihn ab, ohne dass dabei Platz frei wird.
        if let Some(old) = tiles.get(key).map_err(|e| e.to_string())? {
            if let Some((_, old_inserted_at, _)) = decode_entry(old.value()) {
                let old_key = index_key(old_inserted_at, key);
                drop(old);
                index.remove(old_key.as_slice()).map_err(|e| e.to_string())?;
            }
        }

        let mut entry = Vec::with_capacity(16 + packed.len());
        entry.extend_from_slice(&expires_at.to_be_bytes());
        entry.extend_from_slice(&inserted_at.to_be_bytes());
        entry.extend_from_slice(packed);
        tiles.insert(key, entry.as_slice()).map_err(|e| e.to_string())?;
        index
            .insert(index_key(inserted_at, key).as_slice(), ())
            .map_err(|e| e.to_string())?;
    }
    txn.commit().map_err(|e| e.to_string())?;
    Ok(())
}

/// Löscht einen abgelaufenen Eintrag. Bewusst ohne `Result`: Ein Fehlschlag
/// beim Aufräumen soll den vorausgehenden Cache-Miss nicht verschlimmern —
/// die Anfrage geht ohnehin gerade an den Landesdienst weiter.
fn remove_blocking(db: &Database, key: &str, inserted_at: u64) {
    let Ok(txn) = db.begin_write() else { return };
    {
        let Ok(mut tiles) = txn.open_table(TILES) else {
            return;
        };
        let Ok(mut index) = txn.open_table(AGE_INDEX) else {
            return;
        };
        let _ = tiles.remove(key);
        let _ = index.remove(index_key(inserted_at, key).as_slice());
    }
    let _ = txn.commit();
}

/// Wirft die ältesten Einträge raus, bis die Datei wieder unter der Grenze
/// liegt oder ein Durchlauf nichts mehr zum Entfernen findet.
fn evict_if_over_limit(db: &Database, path: &Path, max_bytes: u64) {
    if max_bytes == 0 {
        return;
    }
    loop {
        let Ok(meta) = std::fs::metadata(path) else {
            return;
        };
        if meta.len() <= max_bytes {
            return;
        }

        let Ok(txn) = db.begin_write() else { return };
        let removed = {
            let Ok(mut index) = txn.open_table(AGE_INDEX) else {
                return;
            };
            let Ok(mut tiles) = txn.open_table(TILES) else {
                return;
            };

            let oldest: Vec<Vec<u8>> = match index.iter() {
                Ok(it) => it
                    .filter_map(|r| r.ok())
                    .take(EVICTION_BATCH)
                    .map(|(k, _)| k.value().to_vec())
                    .collect(),
                Err(_) => return,
            };
            for idx_key in &oldest {
                if let Some(cache_key) = key_from_index(idx_key) {
                    let _ = tiles.remove(cache_key);
                }
                let _ = index.remove(idx_key.as_slice());
            }
            oldest.len()
        };
        let _ = txn.commit();

        tracing::debug!(entfernt = removed, "Cache über Grenze — älteste Kacheln verworfen");
        if removed == 0 {
            return;
        }
    }
}

fn index_key(inserted_at: u64, cache_key: &str) -> Vec<u8> {
    let mut v = Vec::with_capacity(8 + cache_key.len());
    v.extend_from_slice(&inserted_at.to_be_bytes());
    v.extend_from_slice(cache_key.as_bytes());
    v
}

fn key_from_index(idx_key: &[u8]) -> Option<&str> {
    idx_key.get(8..).and_then(|b| std::str::from_utf8(b).ok())
}

/// Zerlegt einen gespeicherten Wert in Ablaufzeit, Einfügezeit und Payload.
fn decode_entry(raw: &[u8]) -> Option<(u64, u64, &[u8])> {
    if raw.len() < 16 {
        return None;
    }
    let expires_at = u64::from_be_bytes(raw[0..8].try_into().ok()?);
    let inserted_at = u64::from_be_bytes(raw[8..16].try_into().ok()?);
    Some((expires_at, inserted_at, &raw[16..]))
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store(max_bytes: u64) -> (RedbStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.redb");
        (RedbStore::open(&path, max_bytes).unwrap(), dir)
    }

    #[tokio::test]
    async fn schreibt_und_liest_zurueck() {
        let (store, _dir) = temp_store(0);
        store.set("k1", b"hallo", Duration::from_secs(60)).await;
        assert_eq!(store.get("k1").await.as_deref(), Some(b"hallo".as_slice()));
    }

    #[tokio::test]
    async fn fehlender_schluessel_ist_none() {
        let (store, _dir) = temp_store(0);
        assert_eq!(store.get("nix").await, None);
    }

    #[tokio::test]
    async fn abgelaufener_eintrag_gilt_als_fehlschlag() {
        let (store, _dir) = temp_store(0);
        store.set("k1", b"hallo", Duration::from_secs(0)).await;
        tokio::time::sleep(Duration::from_millis(1_100)).await;
        assert_eq!(store.get("k1").await, None);
    }

    #[tokio::test]
    async fn ueberschreiben_ersetzt_den_wert() {
        let (store, _dir) = temp_store(0);
        store.set("k1", b"alt", Duration::from_secs(60)).await;
        store.set("k1", b"neu", Duration::from_secs(60)).await;
        assert_eq!(store.get("k1").await.as_deref(), Some(b"neu".as_slice()));
    }

    #[tokio::test]
    async fn verdraengt_die_aelteste_kachel_zuerst() {
        // Klein genug, dass ein paar Kilobyte an Nutzdaten die Grenze reißen,
        // aber nicht so klein, dass jeder einzelne Schreibvorgang sich selbst
        // sofort wieder verdrängt.
        let (store, _dir) = temp_store(200_000);
        let payload = vec![0u8; 60_000];
        for i in 0..6 {
            store
                .set(&format!("k{i}"), &payload, Duration::from_secs(60))
                .await;
        }
        assert!(store.get("k0").await.is_none(), "älteste Kachel hätte weichen müssen");
        assert!(store.get("k5").await.is_some(), "jüngste Kachel sollte bleiben");
    }

    #[test]
    fn zstd_komprimiert_geojson_deutlich() {
        // Repräsentativer Ausschnitt: Koordinatenlisten mit viel Wiederholung.
        let json = r#"{"type":"Feature","geometry":{"type":"MultiPolygon","coordinates":
            [[[[356447.097,5644891.501],[356447.475,5644891.435],[356448.128,5644909.273],
            [356448.427,5644909.241],[356449.614,5644923.488],[356449.300,5644923.376]]]]}}"#
            .repeat(20);
        let packed = zstd::encode_all(json.as_bytes(), ZSTD_LEVEL).unwrap();
        let quote = packed.len() as f64 / json.len() as f64;
        assert!(quote < 0.25, "nur auf {:.0} % komprimiert", quote * 100.0);
        let back = zstd::decode_all(packed.as_slice()).unwrap();
        assert_eq!(back, json.as_bytes());
    }
}
