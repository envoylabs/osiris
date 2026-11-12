use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

fn temp_store() -> EvidenceStore {
    static N: AtomicU64 = AtomicU64::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ris-evidence-test-{}-{n}", std::process::id()));
    // Process ids are reused; start from an empty store.
    let _ = std::fs::remove_dir_all(&dir);
    EvidenceStore::open(dir).unwrap()
}

#[test]
fn a_blob_round_trips_and_is_named_by_its_hash() {
    let store = temp_store();
    let id = store.put(b"test output\nok\n").unwrap();
    assert_eq!(id, EvidenceStore::id_of(b"test output\nok\n"));
    assert_eq!(store.get(id).unwrap().unwrap(), b"test output\nok\n");
    assert!(store.has(id));
    // The known SHA-256 of the empty string pins the hash function.
    assert_eq!(
        EvidenceStore::id_of(b"").0.to_hex(),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
}

#[test]
fn putting_the_same_content_twice_is_one_blob() {
    let store = temp_store();
    let a = store.put(b"same").unwrap();
    let b = store.put(b"same").unwrap();
    assert_eq!(a, b);
    assert_eq!(fs::read_dir(&store.dir).unwrap().count(), 1);
}

#[test]
fn a_missing_blob_is_none_and_a_tampered_one_is_an_error() {
    let store = temp_store();
    let absent = EvidenceId(Hash([9; 32]));
    assert!(store.get(absent).unwrap().is_none());
    let id = store.put(b"original").unwrap();
    fs::write(store.path_of(id), b"tampered").unwrap();
    assert_eq!(
        store.get(id).unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
}
